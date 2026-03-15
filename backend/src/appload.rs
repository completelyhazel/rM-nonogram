use std::io::{self, Read, Write};
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixStream;
use std::ffi::CString;
use libc::{socket, connect, AF_UNIX, SOCK_SEQPACKET, SOCK_STREAM, sockaddr_un, socklen_t, c_int, close};
use serde::{Deserialize, Serialize};

pub struct AppLoadConnection {
    stream: UnixStream,
}

#[derive(Debug)]
pub struct Message {
    pub msg_type: u32,
    pub contents: String,
}

#[derive(Deserialize)]
struct RawMessage {
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

    let c_path = CString::new(path).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
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
    let ret = unsafe { connect(fd, &addr as *const sockaddr_un as *const libc::sockaddr, len) };

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
        let fd = match connect_unix(path, SOCK_SEQPACKET) {
            Ok(fd) => { eprintln!("[nonogram-fetcher] conectado via SOCK_SEQPACKET"); fd }
            Err(e) => {
                eprintln!("[nonogram-fetcher] SEQPACKET falló ({}), intentando STREAM…", e);
                connect_unix(path, SOCK_STREAM)?
            }
        };
        Ok(Self { stream: unsafe { UnixStream::from_raw_fd(fd) } })
    }

    pub fn read_message(&mut self) -> Result<Message, Box<dyn std::error::Error>> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf)?;
        let raw: RawMessage = serde_json::from_slice(&buf)?;
        Ok(Message { msg_type: raw.msg_type, contents: raw.contents })
    }

    pub fn send_message(&mut self, msg_type: u32, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
        let json  = serde_json::to_string(&RawOut { msg_type, contents })?;
        let bytes = json.as_bytes();
        self.stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
        self.stream.write_all(bytes)?;
        self.stream.flush()?;
        Ok(())
    }
}
