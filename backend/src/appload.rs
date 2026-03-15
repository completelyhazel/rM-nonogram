// ============================================================================
//  appload.rs — Protocolo de comunicación con AppLoad via Unix socket
//
//  Formato de mensaje (ambas direcciones):
//    [4 bytes little-endian = longitud N] [N bytes = JSON UTF-8]
//
//  JSON:
//    { "type": <u32>, "contents": "<string>" }
// ============================================================================

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
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

impl AppLoadConnection {
    pub fn connect(path: &str) -> io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        Ok(Self { stream })
    }

    pub fn read_message(&mut self) -> Result<Message, Box<dyn std::error::Error>> {
        // Leer 4 bytes de longitud
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;

        // Leer el JSON
        let mut json_buf = vec![0u8; len];
        self.stream.read_exact(&mut json_buf)?;

        let raw: RawMessage = serde_json::from_slice(&json_buf)?;
        Ok(Message {
            msg_type: raw.msg_type,
            contents: raw.contents,
        })
    }

    pub fn send_message(&mut self, msg_type: u32, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string(&RawOut { msg_type, contents })?;
        let bytes = json.as_bytes();
        let len   = bytes.len() as u32;

        self.stream.write_all(&len.to_le_bytes())?;
        self.stream.write_all(bytes)?;
        self.stream.flush()?;
        Ok(())
    }
}
