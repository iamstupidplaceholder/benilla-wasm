//! N1-only binary WebSocket echo. No upstream socket, credentials, or game packets.
//! The synchronous loop has no unbounded queue: it reads one capped frame, writes
//! its echo, and only then accepts the next frame.

use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

use anyhow::{bail, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use sha1::{Digest, Sha1};

use crate::{write_text, Method, Request};

pub(super) const ROUTE: &str = "/game/v1/echo";
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
pub(super) const MAX_MESSAGE: u64 = 64 * 1024;
const MAX_MESSAGES: usize = 64;
// Drain a *small* overshoot so a browser receives close 1009 instead of a
// TCP reset caused by closing with unread frame bytes. Never drain arbitrary
// advertised lengths: a malicious peer must not hold an asset worker forever.
const MAX_REJECT_DRAIN: u64 = MAX_MESSAGE + 4096;

pub(super) fn serve(stream: &mut TcpStream, request: &Request) -> Result<()> {
    if !upgrade(stream, request)? {
        return Ok(());
    }

    if let Err(error) = echo_loop(stream) {
        // An upgraded connection must never fall through to the HTTP worker's
        // 500 response path, even if the browser disconnects during a write.
        eprintln!("N1 echo transport ended: {error:#}");
    }
    Ok(())
}

pub(super) fn upgrade(stream: &mut TcpStream, request: &Request) -> Result<bool> {
    let Some(key) = valid_key(request) else {
        write_text(
            stream,
            400,
            "Bad Request",
            "invalid WebSocket upgrade\n",
            request.method,
        )?;
        return Ok(false);
    };
    let accept = STANDARD.encode(Sha1::digest(format!("{key}{GUID}").as_bytes()));
    write!(stream, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n")?;
    stream.flush()?;

    Ok(true)
}

fn echo_loop(stream: &mut TcpStream) -> Result<()> {
    let mut count = 0;
    loop {
        match read_frame(stream) {
            Ok(Some(Frame { opcode: 2, payload })) => {
                count += 1;
                write_frame(stream, 2, &payload)?;
                if count >= MAX_MESSAGES {
                    write_frame(stream, 8, &1008_u16.to_be_bytes())?;
                    break;
                }
            }
            Ok(Some(Frame { opcode: 9, payload })) => write_frame(stream, 10, &payload)?,
            Ok(Some(Frame { opcode: 8, .. })) => {
                write_frame(stream, 8, &1000_u16.to_be_bytes())?;
                break;
            }
            Ok(Some(_)) => {
                write_frame(stream, 8, &1003_u16.to_be_bytes())?;
                break;
            }
            Ok(None) => break,
            Err(FrameError::TooLarge { drained }) => {
                let _ = write_frame(stream, 8, &1009_u16.to_be_bytes());
                if drained {
                    // Complete the WebSocket close handshake without delaying the
                    // HTTP worker for the normal 10-second request timeout.
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                    let _ = read_frame(stream);
                }
                break;
            }
            Err(FrameError::Protocol) => {
                let _ = write_frame(stream, 8, &1002_u16.to_be_bytes());
                break;
            }
            Err(FrameError::Io(error)) => {
                // The peer may simply have closed or timed out. No HTTP response
                // can be written after a successful WebSocket upgrade.
                eprintln!("N1 echo read ended: {error}");
                break;
            }
        }
    }
    println!("N1 echo ended: {count} binary messages");
    Ok(())
}

pub(super) fn valid_key(request: &Request) -> Option<&str> {
    if request.method != Method::Get
        || request.websocket_version.as_deref() != Some("13")
        || !request
            .upgrade
            .as_deref()?
            .eq_ignore_ascii_case("websocket")
        || !request
            .connection
            .as_deref()?
            .split(',')
            .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
    {
        return None;
    }
    let host = request.host.as_deref()?;
    let origin = request.origin.as_deref()?;
    if origin != format!("http://{host}") && origin != format!("https://{host}") {
        return None;
    }
    let key = request.websocket_key.as_deref()?;
    (STANDARD.decode(key).ok()?.len() == 16).then_some(key)
}

pub(super) struct Frame {
    pub(super) opcode: u8,
    pub(super) payload: Vec<u8>,
}

pub(super) enum FrameError {
    TooLarge { drained: bool },
    Protocol,
    Io(std::io::Error),
}

impl From<std::io::Error> for FrameError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub(super) fn read_frame(stream: &mut TcpStream) -> Result<Option<Frame>, FrameError> {
    let mut header = [0_u8; 2];
    match stream.read(&mut header[..1])? {
        0 => return Ok(None),
        _ => stream.read_exact(&mut header[1..])?,
    }
    let opcode = header[0] & 0x0f;
    if header[0] & 0x70 != 0 || header[0] & 0x80 == 0 || header[1] & 0x80 == 0 {
        return Err(FrameError::Protocol);
    }
    if !matches!(opcode, 2 | 8 | 9) {
        return Err(FrameError::Protocol);
    }
    let mut length = u64::from(header[1] & 0x7f);
    if length == 126 {
        let mut bytes = [0_u8; 2];
        stream.read_exact(&mut bytes)?;
        length = u64::from(u16::from_be_bytes(bytes));
    } else if length == 127 {
        let mut bytes = [0_u8; 8];
        stream.read_exact(&mut bytes)?;
        length = u64::from_be_bytes(bytes);
    }
    if length > MAX_MESSAGE {
        if length <= MAX_REJECT_DRAIN {
            drain_frame_body(stream, length)?;
            return Err(FrameError::TooLarge { drained: true });
        }
        return Err(FrameError::TooLarge { drained: false });
    }
    if matches!(opcode, 8 | 9) && length > 125 {
        return Err(FrameError::Protocol);
    }
    let mut mask = [0_u8; 4];
    stream.read_exact(&mut mask)?;
    let mut payload = vec![0_u8; length as usize];
    stream.read_exact(&mut payload)?;
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % 4];
    }
    Ok(Some(Frame { opcode, payload }))
}

fn drain_frame_body(stream: &mut TcpStream, length: u64) -> std::io::Result<()> {
    let mut mask = [0_u8; 4];
    stream.read_exact(&mut mask)?;
    let mut remaining = length;
    let mut scratch = [0_u8; 4096];
    while remaining != 0 {
        let take = remaining.min(scratch.len() as u64) as usize;
        stream.read_exact(&mut scratch[..take])?;
        remaining -= take as u64;
    }
    Ok(())
}

pub(super) fn write_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> Result<()> {
    if payload.len() > MAX_MESSAGE as usize {
        bail!("N1 echo output exceeds limit");
    }
    stream.write_all(&[0x80 | opcode])?;
    match payload.len() {
        0..=125 => stream.write_all(&[payload.len() as u8])?,
        126..=65535 => {
            stream.write_all(&[126])?;
            stream.write_all(&(payload.len() as u16).to_be_bytes())?;
        }
        _ => {
            stream.write_all(&[127])?;
            stream.write_all(&(payload.len() as u64).to_be_bytes())?;
        }
    }
    stream.write_all(payload)?;
    Ok(())
}
