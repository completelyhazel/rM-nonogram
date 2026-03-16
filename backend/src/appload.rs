// AppLoad IPC — Unix socket bridge between the Rust backend and the QML frontend.
//
// SOCK_SEQPACKET: each send()/recv() is one complete, atomic message.
// No need for a length prefix on the wire — the kernel preserves message
// boundaries for us.
//
// Wire format (both directions):
//   A single datagram containing UTF-8 JSON:  {"type":<u32>,"contents":"<str>"}
//
// Message types (backend → frontend):
//   1  — success  ("SAVED:<path>")
//   2  — error    (human-readable description)
//   3  — progress (status text while working)
//
// Message types (frontend → backend):
//   0  — fetch request (JSON-encoded FetchRequest as the contents string)

use std::io::{self, Write};
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixStream;
use std::ffi::CString;
use libc::{
    socket, connect, recv, AF_UNIX, SOCK_SEQPACKET, SOCK_STREAM,
    sockaddr_un, socklen_t, c_int, close,
};
use serde::{Deserialize, Serialize};

pub struct AppLoadConnection {
    fd:     i32,
    stream: Option<UnixStream>, // only used in STREAM fallback mode
    mode:   SocketMode,
}

#[derive(Clone, Copy)]
enum SocketMode { SeqPacket, Stream }

#[derive(Debug)]
pub struct Message {
    pub msg_type: u32,
    pub contents: String,
}

// ── Wire types ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct IncomingMsg {
    #[serde(rename = "type")]
    msg_type: u32,
    contents: String,
}

#[derive(Serialize)]
struct OutgoingMsg<'a> {
    #[serde(rename = "type")]
    msg_type: u32,
    contents: &'a str,
}

// ── Socket helpers ────────────────────────────────────────────────────────────

fn connect_unix(path: &str, sock_type: c_int) -> io::Result<i32> {
    let fd = unsafe { socket(AF_UNIX, sock_type, 0) };
    if fd < 0 { return Err(io::Error::last_os_error()); }

    let c_path = CString::new(path)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let bytes = c_path.as_bytes_with_nul();
    if bytes.len() > 108 {
        unsafe { close(fd); }
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "socket path too long"));
    }

    let mut addr: sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = AF_UNIX as u16;
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr() as *const libc::c_char,
            addr.sun_path.as_mut_ptr(),
            bytes.len(),
        );
    }

    let addr_len = (std::mem::size_of::<libc::sa_family_t>() + bytes.len()) as socklen_t;
    let ret = unsafe {
        connect(fd, &addr as *const sockaddr_un as *const libc::sockaddr, addr_len)
    };

    if ret < 0 {
        let err = io::Error::last_os_error();
        unsafe { close(fd); }
        Err(err)
    } else {
        Ok(fd)
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

impl AppLoadConnection {
    pub fn connect(path: &str) -> io::Result<Self> {
        match connect_unix(path, SOCK_SEQPACKET) {
            Ok(fd) => {
                eprintln!("[appload] connected via SOCK_SEQPACKET");
                // 300 ms recv timeout so the main loop can drain the mpsc channel
                let tv = libc::timeval { tv_sec: 0, tv_usec: 300_000 };
                unsafe {
                    libc::setsockopt(
                        fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO,
                        &tv as *const _ as *const libc::c_void,
                        std::mem::size_of::<libc::timeval>() as libc::socklen_t,
                    );
                }
                Ok(Self { fd, stream: None, mode: SocketMode::SeqPacket })
            }
            Err(e) => {
                eprintln!("[appload] SEQPACKET failed ({}), falling back to STREAM…", e);
                let fd = connect_unix(path, SOCK_STREAM)?;
                eprintln!("[appload] connected via SOCK_STREAM");
                let stream = unsafe { UnixStream::from_raw_fd(fd) };
                Ok(Self { fd: -1, stream: Some(stream), mode: SocketMode::Stream })
            }
        }
    }

    /// Block until a complete message arrives (or the recv timeout fires).
    pub fn read_message(&mut self) -> Result<Message, Box<dyn std::error::Error>> {
        match self.mode {
            SocketMode::SeqPacket => self.read_seqpacket(),
            SocketMode::Stream    => self.read_stream(),
        }
    }

    /// Send one message to the frontend.
    ///
    /// Protocol: a **single** datagram/write containing UTF-8 JSON:
    ///   {"type":<msg_type>,"contents":"<contents>"}
    ///
    /// SEQPACKET guarantees the receiver gets exactly this many bytes in one
    /// recv() call, so no length prefix is needed.
    pub fn send_message(&mut self, msg_type: u32, contents: &str)
        -> Result<(), Box<dyn std::error::Error>>
    {
        let json = serde_json::to_string(&OutgoingMsg { msg_type, contents })?;
        let bytes = json.as_bytes();

        eprintln!("[appload] send type={} contents={:?}", msg_type, contents);

        match self.mode {
            SocketMode::SeqPacket => {
                let sent = unsafe {
                    libc::send(
                        self.fd,
                        bytes.as_ptr() as *const libc::c_void,
                        bytes.len(),
                        0,
                    )
                };
                if sent < 0 {
                    return Err(io::Error::last_os_error().into());
                }
            }
            SocketMode::Stream => {
                // STREAM needs an explicit length prefix so the reader knows
                // where the message ends.
                let len = bytes.len() as u32;
                let stream = self.stream.as_mut().unwrap();
                stream.write_all(&len.to_le_bytes())?;
                stream.write_all(bytes)?;
                stream.flush()?;
            }
        }
        Ok(())
    }
}

// ── Private read helpers ──────────────────────────────────────────────────────

impl AppLoadConnection {
    fn read_seqpacket(&mut self) -> Result<Message, Box<dyn std::error::Error>> {
        let mut buf = vec![0u8; 65_536];
        let n = unsafe {
            recv(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0)
        };

        if n <= 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock
                || err.kind() == io::ErrorKind::TimedOut
            {
                return Err("timeout".into());
            }
            return Err(format!("recv returned {}: {}", n, err).into());
        }

        let n = n as usize;
        eprintln!("[appload] received {} bytes", n);

        // AppLoad may send the payload with or without a 4-byte LE length
        // prefix. Try both so we stay compatible with different AppLoad builds.
        let payload = if n >= 4 {
            let declared_len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
            if declared_len + 4 == n {
                &buf[4..n] // length-prefixed
            } else {
                &buf[..n]  // raw JSON
            }
        } else {
            &buf[..n]
        };

        eprintln!("[appload] payload: {}", String::from_utf8_lossy(payload));

        // Try the standard wrapped format first.
        if let Ok(msg) = serde_json::from_slice::<IncomingMsg>(payload) {
            return Ok(Message { msg_type: msg.msg_type, contents: msg.contents });
        }

        // Fall back: AppLoad sent the contents string directly (no wrapper).
        // Infer type from content.
        let contents = String::from_utf8_lossy(payload).trim().to_string();
        let msg_type = if contents.contains("type_bw") { 0 } else { 99 };
        Ok(Message { msg_type, contents })
    }

    fn read_stream(&mut self) -> Result<Message, Box<dyn std::error::Error>> {
        use std::io::Read;
        let stream = self.stream.as_mut().unwrap();

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;

        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf)?;

        eprintln!("[appload] received {} bytes (stream)", len);
        eprintln!("[appload] payload: {}", String::from_utf8_lossy(&buf));

        let msg: IncomingMsg = serde_json::from_slice(&buf)?;
        Ok(Message { msg_type: msg.msg_type, contents: msg.contents })
    }
}