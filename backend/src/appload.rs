
use std::io::{self, Write};
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixStream;
use std::ffi::CString;
use libc::{
    socket, connect, recv, AF_UNIX, SOCK_SEQPACKET, SOCK_STREAM,
    sockaddr_un, socklen_t, c_int, close,
};
use serde::Deserialize;

pub struct AppLoadConnection {
    fd:     i32,
    stream: Option<UnixStream>,
    mode:   SocketMode,
}

#[derive(Clone, Copy)]
enum SocketMode { SeqPacket, Stream }

#[derive(Debug)]
pub struct Message {
    pub msg_type: u32,
    pub contents: String,
}

/// fallback JSON wrapper used by some AppLoad versions
#[derive(Deserialize)]
struct JsonMsg {
    #[serde(rename = "type")]
    msg_type: u32,
    contents: String,
}

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

    let len = (std::mem::size_of::<libc::sa_family_t>() + bytes.len()) as socklen_t;
    let ret = unsafe {
        connect(fd, &addr as *const sockaddr_un as *const libc::sockaddr, len)
    };

    if ret < 0 {
        let err = io::Error::last_os_error();
        unsafe { close(fd); }
        Err(err)
    } else {
        Ok(fd)
    }
}

impl AppLoadConnection {
    pub fn connect(path: &str) -> io::Result<Self> {
        match connect_unix(path, SOCK_SEQPACKET) {
            Ok(fd) => {
                eprintln!("[fetcher] connected via SOCK_SEQPACKET");
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
                eprintln!("[fetcher] SEQPACKET failed ({}), trying STREAM…", e);
                let fd = connect_unix(path, SOCK_STREAM)?;
                eprintln!("[fetcher] connected via SOCK_STREAM");
                let stream = unsafe { UnixStream::from_raw_fd(fd) };
                Ok(Self { fd: -1, stream: Some(stream), mode: SocketMode::Stream })
            }
        }
    }

    pub fn read_message(&mut self) -> Result<Message, Box<dyn std::error::Error>> {
        match self.mode {
            SocketMode::SeqPacket => self.read_seqpacket(),
            SocketMode::Stream    => self.read_stream(),
        }
    }
    fn read_seqpacket(&mut self) -> Result<Message, Box<dyn std::error::Error>> {
        let mut buf = vec![0u8; 65536];

        // ---!!!!! receive first datagram (header or legacy single-datagram message) ---
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
            return Err(format!("recv returned {n}: {err}").into());
        }
        let n = n as usize;

        if n == 8 {
            let msg_type    = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
            let content_len = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;

            if content_len == 0 {
                eprintln!("[fetcher] received type={msg_type} (empty content)");
                return Ok(Message { msg_type, contents: String::new() });
            }

            // !!!! receive second datagram (content)
            let mut cbuf = vec![0u8; content_len.min(65536)];
            let cn = unsafe {
                recv(self.fd, cbuf.as_mut_ptr() as *mut libc::c_void, cbuf.len(), 0)
            };
            if cn <= 0 {
                return Err(format!(
                    "recv content failed: {}", io::Error::last_os_error()
                ).into());
            }
            let contents = String::from_utf8_lossy(&cbuf[..cn as usize])
                .trim()
                .to_string();
            eprintln!(
                "[fetcher] received type={msg_type} contents={:?}",
                &contents[..contents.len().min(120)]
            );
            return Ok(Message { msg_type, contents });
        }

        // fallback
        let slice = &buf[..n];
        eprintln!("[fetcher] single datagram {n} bytes");

        // try JSON wrapper {"type": N, "contents": "..."}
        if let Ok(jm) = serde_json::from_slice::<JsonMsg>(slice) {
            return Ok(Message { msg_type: jm.msg_type, contents: jm.contents });
        }

        // raw text n detect type from content as a last resort
        let contents = String::from_utf8_lossy(slice).trim().to_string();
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
        eprintln!("[fetcher] stream message: {}", String::from_utf8_lossy(&buf));
        let jm: JsonMsg = serde_json::from_slice(&buf)?;
        Ok(Message { msg_type: jm.msg_type, contents: jm.contents })
    }

    pub fn send_message(
        &mut self,
        msg_type: u32,
        contents: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let c_bytes = contents.as_bytes();
        let mut header = [0u8; 8];
        header[0..4].copy_from_slice(&msg_type.to_le_bytes());
        header[4..8].copy_from_slice(&(c_bytes.len() as u32).to_le_bytes());

        eprintln!(
            "[fetcher] send type={msg_type} len={} contents={:?}",
            c_bytes.len(),
            &contents[..contents.len().min(80)]
        );

        match self.mode {
            SocketMode::SeqPacket => {
                let s1 = unsafe {
                    libc::send(self.fd, header.as_ptr() as *const libc::c_void, 8, 0)
                };
                if s1 < 0 { return Err(io::Error::last_os_error().into()); }

                let s2 = unsafe {
                    libc::send(
                        self.fd,
                        c_bytes.as_ptr() as *const libc::c_void,
                        c_bytes.len(),
                        0,
                    )
                };
                if s2 < 0 { return Err(io::Error::last_os_error().into()); }
            }
            SocketMode::Stream => {
                let stream = self.stream.as_mut().unwrap();
                stream.write_all(&header)?;
                stream.write_all(c_bytes)?;
                stream.flush()?;
            }
        }
        Ok(())
    }
}
