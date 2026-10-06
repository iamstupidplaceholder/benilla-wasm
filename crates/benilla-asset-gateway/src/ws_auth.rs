//! N2/N3 game transport: an opaque, ordered byte relay to one server-configured
//! TCP endpoint per route. This does not parse or log credentials or packets.

use std::{
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use anyhow::{bail, Result};

use crate::ws_echo::{self, FrameError};
use crate::{write_text, Request};

pub(super) const AUTH_ROUTE: &str = "/game/v1/auth";
pub(super) const WORLD_ROUTE: &str = "/game/v1/world";
pub(super) fn serve(
    stream: &mut TcpStream,
    request: &Request,
    upstream_addr: SocketAddr,
    max_session_bytes: Option<usize>,
) -> Result<()> {
    if ws_echo::valid_key(request).is_none() {
        let _ = ws_echo::upgrade(stream, request)?;
        return Ok(());
    }
    let upstream = match TcpStream::connect_timeout(&upstream_addr, Duration::from_secs(3)) {
        Ok(stream) => stream,
        Err(_) => {
            write_text(
                stream,
                502,
                "Bad Gateway",
                "configured game server is unavailable\n",
                request.method,
            )?;
            return Ok(());
        }
    };
    upstream.set_nodelay(true)?;
    let upstream_write = upstream.try_clone()?;
    let websocket_write = Arc::new(Mutex::new(stream.try_clone()?));
    if !ws_echo::upgrade(stream, request)? {
        return Ok(());
    }
    stream.set_read_timeout(Some(Duration::from_secs(90)))?;

    let stopped = Arc::new(AtomicBool::new(false));
    let worker_writer = websocket_write.clone();
    let worker_stopped = stopped.clone();
    let worker = std::thread::Builder::new()
        .name("game-relay-upstream".into())
        .spawn(move || relay_upstream(upstream, worker_writer, worker_stopped, max_session_bytes));
    let worker = match worker {
        Ok(worker) => worker,
        Err(_) => {
            let _ = write_ws(&websocket_write, 8, &1011_u16.to_be_bytes());
            return Ok(());
        }
    };

    let result = relay_browser(
        stream,
        &upstream_write,
        &websocket_write,
        &stopped,
        max_session_bytes,
    );
    stopped.store(true, Ordering::Relaxed);
    let _ = upstream_write.shutdown(Shutdown::Both);
    if worker.join().is_err() {
        eprintln!("game relay worker panicked");
    }
    if let Err(error) = result {
        // Deliberately do not print raw packets or account/SRP data.
        eprintln!("game relay ended: {error:#}");
    }
    Ok(())
}

fn relay_browser(
    websocket_read: &mut TcpStream,
    upstream_write: &TcpStream,
    websocket_write: &Arc<Mutex<TcpStream>>,
    stopped: &AtomicBool,
    max_session_bytes: Option<usize>,
) -> Result<()> {
    let mut upstream_write = upstream_write;
    let mut bytes = 0usize;
    loop {
        match ws_echo::read_frame(websocket_read) {
            Ok(Some(frame)) if frame.opcode == 2 => {
                bytes = bytes.saturating_add(frame.payload.len());
                if max_session_bytes.is_some_and(|limit| bytes > limit) {
                    let _ = write_ws(websocket_write, 8, &1009_u16.to_be_bytes());
                    break;
                }
                upstream_write.write_all(&frame.payload)?;
            }
            Ok(Some(frame)) if frame.opcode == 9 => {
                write_ws(websocket_write, 10, &frame.payload)?;
            }
            Ok(Some(frame)) if frame.opcode == 8 => {
                let _ = write_ws(websocket_write, 8, &1000_u16.to_be_bytes());
                break;
            }
            Ok(Some(_)) => {
                let _ = write_ws(websocket_write, 8, &1003_u16.to_be_bytes());
                break;
            }
            Ok(None) => break,
            Err(FrameError::TooLarge { drained }) => {
                let _ = write_ws(websocket_write, 8, &1009_u16.to_be_bytes());
                if drained {
                    let _ = websocket_read.set_read_timeout(Some(Duration::from_millis(500)));
                    let _ = ws_echo::read_frame(websocket_read);
                }
                break;
            }
            Err(FrameError::Protocol) => {
                let _ = write_ws(websocket_write, 8, &1002_u16.to_be_bytes());
                break;
            }
            Err(FrameError::Io(_)) if stopped.load(Ordering::Relaxed) => break,
            Err(FrameError::Io(error)) => return Err(error.into()),
        }
    }
    Ok(())
}

fn relay_upstream(
    mut upstream: TcpStream,
    websocket_write: Arc<Mutex<TcpStream>>,
    stopped: Arc<AtomicBool>,
    max_session_bytes: Option<usize>,
) {
    if upstream
        .set_read_timeout(Some(Duration::from_millis(250)))
        .is_err()
    {
        stopped.store(true, Ordering::Relaxed);
        return;
    }
    let mut bytes = 0usize;
    let mut chunk = [0_u8; 4096];
    while !stopped.load(Ordering::Relaxed) {
        match upstream.read(&mut chunk) {
            Ok(0) => {
                if !stopped.load(Ordering::Relaxed) {
                    let _ = write_ws(&websocket_write, 8, &1000_u16.to_be_bytes());
                }
                break;
            }
            Ok(length) => {
                bytes = bytes.saturating_add(length);
                if max_session_bytes.is_some_and(|limit| bytes > limit) {
                    let _ = write_ws(&websocket_write, 8, &1009_u16.to_be_bytes());
                    break;
                }
                if write_ws(&websocket_write, 2, &chunk[..length]).is_err() {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                continue
            }
            Err(_) => break,
        }
    }
    stopped.store(true, Ordering::Relaxed);
    if let Ok(socket) = websocket_write.lock() {
        let _ = socket.shutdown(Shutdown::Read);
    }
}

fn write_ws(stream: &Arc<Mutex<TcpStream>>, opcode: u8, payload: &[u8]) -> Result<()> {
    let mut stream = stream
        .lock()
        .map_err(|_| anyhow::anyhow!("game websocket writer poisoned"))?;
    if payload.len() > ws_echo::MAX_MESSAGE as usize {
        bail!("game websocket frame exceeds limit");
    }
    ws_echo::write_frame(&mut stream, opcode, payload)
}
