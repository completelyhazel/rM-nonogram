// Con SOCK_SEQPACKET cada recv() lee UN mensaje completo de una vez.
// No se puede hacer read_exact en dos pasos (4 bytes len + payload).
// Solución: recv() en un buffer grande, parsear los 4 bytes del inicio.

use std::io::{self, Write};
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixStream;
use std::ffi::CString;
use libc::{
    socket, connect, recv, AF_UNIX, SOCK_SEQPACKET, SOCK_STREAM,
    sockaddr_un, socklen_t, c_int, close, MSG_WAITALL,
};
use serde::{Deserialize, Serialize};

pub struct AppLoadConnection {
    fd:     i32,
    stream: Option<UnixStream>,  // solo usado para enviar (STREAM mode)
    mode:   SocketMode,
}

#[derive(Clone, Copy)]
enum SocketMode { SeqPacket, Stream }

#[derive(Debug)]
pub struct Message {
    pub msg_type: u32,
    pub contents: String,
}

#[derive(Deserialize)]
struct RawMsg {
    #[serde(rename = "type")]
    msg_type: u32,
    contents: String,
}

#[derive(Serialize)]
struct RawOut<'a> {
    #[serde(rename = "type")]
    msg_type: u32,
    contents: &'a str,
}

fn connect_unix(path: &str, sock_type: c_int) -> io::Result<i32> {
    let fd = unsafe { socket(AF_UNIX, sock_type, 0) };
    if fd < 0 { return Err(io::Error::last_os_error()); }

    let c_path = CString::new(path)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let bytes = c_path.as_bytes_with_nul();
    if bytes.len() > 108 {
        unsafe { close(fd); }
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "path too long"));
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
                eprintln!("[nonogram-fetcher] conectado via SOCK_SEQPACKET");
                // SO_RCVTIMEO: timeout 300ms para que el loop pueda vaciar el canal mpsc
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
                eprintln!("[nonogram-fetcher] SEQPACKET falló ({}), intentando STREAM…", e);
                let fd = connect_unix(path, SOCK_STREAM)?;
                eprintln!("[nonogram-fetcher] conectado via SOCK_STREAM");
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
        // Un solo recv() lee el mensaje completo
        let mut buf = vec![0u8; 65536];
        let n = unsafe {
            recv(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0)
        };
        if n <= 0 {
            let err = io::Error::last_os_error();
            // EAGAIN/EWOULDBLOCK = timeout expiró, no hay mensaje todavía
            if err.kind() == io::ErrorKind::WouldBlock || err.kind() == io::ErrorKind::TimedOut {
                return Err("timeout".into());
            }
            return Err(format!("recv devolvió {}: {}", n, err).into());
        }
        let n = n as usize;
        eprintln!("[nonogram-fetcher] recibidos {} bytes via SEQPACKET", n);

        // El mensaje puede tener 4 bytes de longitud al inicio, o ser JSON directo
        // AppLoad SEQPACKET envía el contents directamente (sin wrapper).
        // Intentar primero formato con prefijo de longitud, luego directo.
        let json_bytes = if n >= 4 {
            let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
            if len + 4 == n { &buf[4..n] } else { &buf[..n] }
        } else {
            &buf[..n]
        };

        let text = String::from_utf8_lossy(json_bytes);
        eprintln!("[nonogram-fetcher] JSON: {}", text);

        // Intentar formato con wrapper {"type":N,"contents":"..."}
        if let Ok(raw) = serde_json::from_slice::<RawMsg>(json_bytes) {
            return Ok(Message { msg_type: raw.msg_type, contents: raw.contents });
        }

        // Formato directo: contents enviado sin wrapper, inferir tipo por contenido
        let contents = text.trim().to_string();
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
        eprintln!("[nonogram-fetcher] JSON (stream): {}", String::from_utf8_lossy(&buf));
        let raw: RawMsg = serde_json::from_slice(&buf)?;
        Ok(Message { msg_type: raw.msg_type, contents: raw.contents })
    }

    pub fn send_message(&mut self, msg_type: u32, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Formato AppLoad (Qt QDataStream, big-endian):
        //   [4 bytes BE = longitud del payload]
        //   [4 bytes BE = tipo]
        //   [N bytes   = contents string]
        let c_bytes = contents.as_bytes();
        let payload_len = (4 + c_bytes.len()) as u32;

        let mut packet = Vec::with_capacity(4 + 4 + c_bytes.len());
        packet.extend_from_slice(&payload_len.to_be_bytes());   // longitud BE
        packet.extend_from_slice(&msg_type.to_be_bytes());      // tipo BE
        packet.extend_from_slice(c_bytes);                      // contents

        eprintln!("[nonogram-fetcher] send {} bytes: len={} type={} contents={:?}",
            packet.len(), payload_len, msg_type, contents);

        match self.mode {
            SocketMode::SeqPacket => {
                let sent = unsafe {
                    libc::send(self.fd, packet.as_ptr() as *const libc::c_void, packet.len(), 0)
                };
                if sent < 0 {
                    return Err(io::Error::last_os_error().into());
                }
            }
            SocketMode::Stream => {
                self.stream.as_mut().unwrap().write_all(&packet)?;
                self.stream.as_mut().unwrap().flush()?;
            }
        }
        Ok(())
    }
}
