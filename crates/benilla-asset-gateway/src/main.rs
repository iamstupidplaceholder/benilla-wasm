//! Development asset gateway for the browser client.
//!
//! It deliberately exposes logical MPQ paths, never host paths. `Chain` remains the authority for
//! patch order, tombstones and decompression; this process only validates, bounds and caches reads.

use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use benilla_formats::{Chain, normalize_client_path};
use clap::{CommandFactory, FromArgMatches, Parser};
use sha1::{Digest, Sha1};

mod addons;
mod config;
mod locales;
mod presence;
mod ws_auth;
mod ws_echo;

const ASSET_ROUTE: &str = "/assets/v1/file/";
const MAX_REQUEST_HEAD: usize = 16 * 1024;

#[derive(Parser, Debug)]
#[command(about = "Serve Benilla Web and logical files from a WoW MPQ patch chain")]
struct Args {
    /// TOML configuration file (automatically reads ./gateway.toml when present).
    #[arg(long)]
    config: Option<PathBuf>,

    /// WoW Data directory. Falls back to Benilla's normal WOW_DATA/install discovery.
    #[arg(long)]
    data: Option<PathBuf>,

    /// Single loose AddOns root. Defaults to Interface/AddOns beside the Data directory.
    #[arg(long)]
    addons: Option<PathBuf>,

    /// Directory containing index.html and pkg/.
    #[arg(long, default_value = "web")]
    web: PathBuf,

    #[arg(long, default_value = "127.0.0.1:8000")]
    bind: String,

    /// Fixed Realmd TCP destination, owned by the gateway configuration.
    #[arg(long, default_value = "127.0.0.1:3724")]
    auth_upstream: SocketAddr,

    /// Fixed World TCP destination, owned by the gateway configuration.
    #[arg(long, default_value = "127.0.0.1:8085")]
    world_upstream: SocketAddr,

    /// Maximum simultaneous request/decompression workers.
    #[arg(long, default_value_t = 4)]
    workers: usize,

    /// Maximum accepted connections waiting for a worker.
    #[arg(long, default_value_t = 16)]
    queue: usize,

    /// Reject a logical asset before decompression when its expanded size exceeds this limit.
    #[arg(long, default_value_t = 64)]
    max_asset_mib: u64,

    /// Maximum decompressed logical-asset cache size.
    #[arg(long, default_value_t = 128)]
    cache_mib: usize,

    /// Browser asset-cache freshness in seconds; zero revalidates every request.
    #[arg(long, default_value_t = 300)]
    asset_max_age_secs: u64,
}

struct Gateway {
    chain: Arc<Chain>,
    addons: addons::LooseAddons,
    presence: CachedAsset,
    macro_icons: CachedAsset,
    locales: locales::Locales,
    web_root: PathBuf,
    max_asset_bytes: u64,
    cache: Mutex<AssetCache>,
    auth_upstream: SocketAddr,
    world_upstream: SocketAddr,
    asset_cache_control: String,
}

#[derive(Clone)]
struct CachedAsset {
    bytes: Arc<Vec<u8>>,
    etag: String,
    archive: String,
}

struct AssetCache {
    entries: HashMap<String, CachedAsset>,
    insertion_order: VecDeque<String>,
    resident_bytes: usize,
    limit_bytes: usize,
}

impl AssetCache {
    fn new(limit_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
            resident_bytes: 0,
            limit_bytes,
        }
    }

    fn get(&self, path: &str) -> Option<CachedAsset> {
        self.entries.get(path).cloned()
    }

    fn insert(&mut self, path: String, asset: CachedAsset) {
        let size = asset.bytes.len();
        if size > self.limit_bytes || self.entries.contains_key(&path) {
            return;
        }
        while self.resident_bytes.saturating_add(size) > self.limit_bytes {
            let Some(oldest) = self.insertion_order.pop_front() else {
                break;
            };
            if let Some(old) = self.entries.remove(&oldest) {
                self.resident_bytes = self.resident_bytes.saturating_sub(old.bytes.len());
            }
        }
        self.resident_bytes = self.resident_bytes.saturating_add(size);
        self.insertion_order.push_back(path.clone());
        self.entries.insert(path, asset);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Get,
    Head,
}

struct Request {
    method: Method,
    target: String,
    if_none_match: Option<String>,
    host: Option<String>,
    origin: Option<String>,
    upgrade: Option<String>,
    connection: Option<String>,
    websocket_key: Option<String>,
    websocket_version: Option<String>,
}

fn main() -> Result<()> {
    let matches = Args::command().get_matches();
    let mut args = Args::from_arg_matches(&matches)?;
    config::apply(&mut args, &matches)?;
    if args.workers == 0 || args.queue == 0 {
        bail!("--workers and --queue must be greater than zero");
    }
    let data = args
        .data
        .or_else(benilla_formats::wow_data)
        .context("no WoW Data directory found; pass --data /path/to/WoW/Data or set WOW_DATA")?;
    if !args.web.join("index.html").is_file() {
        bail!("{} does not contain index.html", args.web.display());
    }

    let chain = Arc::new(
        Chain::open(&data).with_context(|| format!("opening patch chain at {}", data.display()))?,
    );
    let gateway = Arc::new(Gateway {
        addons: addons::LooseAddons::open(
            args.addons,
            &data,
            args.max_asset_mib.saturating_mul(1024 * 1024),
        )?,
        presence: presence::build(&chain, args.max_asset_mib.saturating_mul(1024 * 1024))?,
        locales: locales::build(
            &data,
            &chain,
            args.max_asset_mib.saturating_mul(1024 * 1024),
        )?,
        macro_icons: {
            let icons = benilla_formats::load_macro_icons(&chain)?;
            let bytes = icons.join("\n").into_bytes();
            CachedAsset {
                etag: format!("\"{:x}\"", Sha1::digest(&bytes)),
                bytes: Arc::new(bytes),
                archive: "macro-icon-enumeration".into(),
            }
        },
        chain,
        web_root: args.web,
        max_asset_bytes: args.max_asset_mib.saturating_mul(1024 * 1024),
        cache: Mutex::new(AssetCache::new(args.cache_mib.saturating_mul(1024 * 1024))),
        auth_upstream: args.auth_upstream,
        world_upstream: args.world_upstream,
        asset_cache_control: format!(
            "public, max-age={}, must-revalidate",
            args.asset_max_age_secs
        ),
    });
    let listener = TcpListener::bind(&args.bind)
        .with_context(|| format!("binding asset gateway to {}", args.bind))?;
    let (sender, receiver) = mpsc::sync_channel::<TcpStream>(args.queue);
    let receiver = Arc::new(Mutex::new(receiver));

    for worker in 0..args.workers {
        let gateway = gateway.clone();
        let receiver = receiver.clone();
        std::thread::Builder::new()
            .name(format!("asset-gateway-{worker}"))
            .spawn(move || worker_loop(gateway, receiver))?;
    }

    println!("Benilla asset gateway: http://{}", args.bind);
    println!("MPQ chain: {}", data.display());
    println!("Web root: {}", gateway.web_root.display());
    println!("Asset route: {ASSET_ROUTE}<logical-client-path>");
    println!("N1 binary WebSocket echo: /game/v1/echo (same-origin probe only)");
    println!("N2a auth relay: /game/v1/auth -> {}", gateway.auth_upstream);
    println!(
        "N3 world relay: /game/v1/world -> {}",
        gateway.world_upstream
    );
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if sender.send(stream).is_err() {
                    break;
                }
            }
            Err(error) => eprintln!("accept error: {error}"),
        }
    }
    Ok(())
}

fn worker_loop(gateway: Arc<Gateway>, receiver: Arc<Mutex<mpsc::Receiver<TcpStream>>>) {
    loop {
        let stream = {
            let Ok(receiver) = receiver.lock() else {
                return;
            };
            receiver.recv()
        };
        let Ok(mut stream) = stream else {
            return;
        };
        if let Err(error) = handle_connection(&gateway, &mut stream) {
            eprintln!("request error: {error:#}");
            let _ = write_bytes(
                &mut stream,
                500,
                "Internal Server Error",
                "text/plain; charset=utf-8",
                b"internal server error\n",
                &[],
                Method::Get,
            );
        }
    }
}

fn handle_connection(gateway: &Gateway, stream: &mut TcpStream) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let request = match read_request(stream) {
        Ok(request) => request,
        Err(error) => {
            write_text(
                stream,
                400,
                "Bad Request",
                &format!("{error}\n"),
                Method::Get,
            )?;
            return Ok(());
        }
    };
    let target = request.target.split('?').next().unwrap_or_default();
    if target == ws_echo::ROUTE {
        // Short-lived probe. N2a below is also dev-only until game sessions
        // have capacity separate from the HTTP asset workers.
        ws_echo::serve(stream, &request)
    } else if target == ws_auth::AUTH_ROUTE {
        serve_game(stream, &request, gateway.auth_upstream, Some(1024 * 1024))
    } else if target == ws_auth::WORLD_ROUTE {
        serve_game(stream, &request, gateway.world_upstream, None)
    } else if target == addons::ROUTE {
        presence::serve(&gateway.addons.catalog, stream, &request)
    } else if target == locales::ROUTE {
        presence::serve(&gateway.locales.catalog, stream, &request)
    } else if let Some(relative) = target.strip_prefix("/assets/v1/locale/") {
        let Some((code, path)) = relative.split_once('/') else {
            return write_text(
                stream,
                400,
                "Bad Request",
                "locale route requires a resource\n",
                request.method,
            );
        };
        let Some(language) = gateway.locales.assets.get(code) else {
            return write_text(
                stream,
                404,
                "Not Found",
                "locale not available\n",
                request.method,
            );
        };
        if let Some(encoded) = path.strip_prefix("file/") {
            serve_asset(gateway, &language.chain, code, stream, &request, encoded)
        } else {
            let asset = match path {
                "presence.bfi" => &language.presence,
                "macro-icons.bfi" => &language.macro_icons,
                "addons.bai" => &gateway.addons.catalog,
                "locales.bli" => &gateway.locales.catalog,
                _ => {
                    return write_text(
                        stream,
                        404,
                        "Not Found",
                        "locale metadata not found\n",
                        request.method,
                    );
                }
            };
            presence::serve(asset, stream, &request)
        }
    } else if let Some(encoded) = target.strip_prefix(ASSET_ROUTE) {
        serve_asset(gateway, &gateway.chain, "enUS", stream, &request, encoded)
    } else if target == presence::ROUTE {
        presence::serve(&gateway.presence, stream, &request)
    } else if target == "/assets/v1/macro-icons.bfi" {
        presence::serve(&gateway.macro_icons, stream, &request)
    } else {
        serve_static(gateway, stream, &request, target)
    }
}

fn serve_game(
    stream: &mut TcpStream,
    request: &Request,
    default: SocketAddr,
    limit: Option<usize>,
) -> Result<()> {
    if request.target.contains('?') {
        return write_text(
            stream,
            400,
            "Bad Request",
            "game relay query parameters are not supported\n",
            request.method,
        );
    }
    ws_auth::serve(stream, request, default, limit)
}

fn read_request(stream: &mut TcpStream) -> Result<Request> {
    let mut head = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            bail!("connection closed before request headers");
        }
        head.extend_from_slice(&chunk[..read]);
        if head.len() > MAX_REQUEST_HEAD {
            bail!("request headers exceed {MAX_REQUEST_HEAD} bytes");
        }
    }
    let head = std::str::from_utf8(&head).context("request headers are not UTF-8")?;
    let mut lines = head.split("\r\n");
    let mut request_line = lines
        .next()
        .context("missing request line")?
        .split_whitespace();
    let method = match request_line.next() {
        Some("GET") => Method::Get,
        Some("HEAD") => Method::Head,
        Some(other) => bail!("unsupported method {other}"),
        None => bail!("missing method"),
    };
    let target = request_line.next().context("missing request target")?;
    if !target.starts_with('/') {
        bail!("request target must be origin-relative");
    }
    let mut if_none_match = None;
    let mut host = None;
    let mut origin = None;
    let mut upgrade = None;
    let mut connection = None;
    let mut websocket_key = None;
    let mut websocket_version = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("if-none-match") {
            if_none_match = Some(value.trim().to_owned());
        } else if name.eq_ignore_ascii_case("host") {
            host = Some(value.trim().to_owned());
        } else if name.eq_ignore_ascii_case("origin") {
            origin = Some(value.trim().to_owned());
        } else if name.eq_ignore_ascii_case("upgrade") {
            upgrade = Some(value.trim().to_owned());
        } else if name.eq_ignore_ascii_case("connection") {
            connection = Some(value.trim().to_owned());
        } else if name.eq_ignore_ascii_case("sec-websocket-key") {
            websocket_key = Some(value.trim().to_owned());
        } else if name.eq_ignore_ascii_case("sec-websocket-version") {
            websocket_version = Some(value.trim().to_owned());
        }
    }
    Ok(Request {
        method,
        target: target.to_owned(),
        if_none_match,
        host,
        origin,
        upgrade,
        connection,
        websocket_key,
        websocket_version,
    })
}

fn serve_asset(
    gateway: &Gateway,
    chain: &Chain,
    locale: &str,
    stream: &mut TcpStream,
    request: &Request,
    encoded: &str,
) -> Result<()> {
    let decoded = match percent_decode(encoded).and_then(|bytes| {
        String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("asset path is not UTF-8"))
    }) {
        Ok(decoded) => decoded,
        Err(error) => {
            write_text(
                stream,
                400,
                "Bad Request",
                &format!("invalid asset path: {error}\n"),
                request.method,
            )?;
            return Ok(());
        }
    };
    let logical = match normalize_client_path(&decoded) {
        Ok(path) => path,
        Err(error) => {
            write_text(
                stream,
                400,
                "Bad Request",
                &format!("invalid asset path: {error}\n"),
                request.method,
            )?;
            return Ok(());
        }
    };

    let cache_key = format!("{locale}:{logical}");
    let cached = gateway
        .cache
        .lock()
        .expect("asset cache poisoned")
        .get(&cache_key);
    let (asset, cache_state) = if let Some(asset) = cached {
        (asset, "HIT")
    } else {
        let loose = gateway.addons.size(&logical);
        let Some(size) = loose.or_else(|| chain.file_size(&logical)) else {
            println!("404 {logical}");
            write_text(
                stream,
                404,
                "Not Found",
                "asset not found\n",
                request.method,
            )?;
            return Ok(());
        };
        if size > gateway.max_asset_bytes {
            println!("413 {logical} ({size} bytes)");
            write_text(
                stream,
                413,
                "Content Too Large",
                "asset exceeds gateway limit\n",
                request.method,
            )?;
            return Ok(());
        }
        let archive = if loose.is_some() {
            "loose-addon".to_owned()
        } else {
            chain
                .find_file_archive(&logical)
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .unwrap_or("unknown")
                .to_owned()
        };
        let bytes = if loose.is_some() {
            gateway.addons.read(&logical, gateway.max_asset_bytes)?
        } else {
            chain.read(&logical)?
        };
        let etag = format!("\"{:x}\"", Sha1::digest(&bytes));
        let asset = CachedAsset {
            bytes: Arc::new(bytes),
            etag,
            archive,
        };
        gateway
            .cache
            .lock()
            .expect("asset cache poisoned")
            .insert(cache_key, asset.clone());
        (asset, "MISS")
    };

    if request.if_none_match.as_deref() == Some(asset.etag.as_str()) {
        println!("304 {logical} <- {}", asset.archive);
        write_bytes(
            stream,
            304,
            "Not Modified",
            "application/octet-stream",
            &[],
            &[
                ("ETag", asset.etag.as_str()),
                ("Cache-Control", gateway.asset_cache_control.as_str()),
            ],
            request.method,
        )?;
        return Ok(());
    }

    println!(
        "200 {cache_state} {logical} ({} bytes) <- {}",
        asset.bytes.len(),
        asset.archive
    );
    write_bytes(
        stream,
        200,
        "OK",
        "application/octet-stream",
        &asset.bytes,
        &[
            ("ETag", asset.etag.as_str()),
            ("Cache-Control", gateway.asset_cache_control.as_str()),
            ("X-Benilla-Archive", asset.archive.as_str()),
        ],
        request.method,
    )?;
    Ok(())
}

fn serve_static(
    gateway: &Gateway,
    stream: &mut TcpStream,
    request: &Request,
    target: &str,
) -> Result<()> {
    let target = if target == "/" { "/index.html" } else { target };
    let relative = match safe_static_path(target) {
        Ok(path) => path,
        Err(error) => {
            write_text(
                stream,
                400,
                "Bad Request",
                &format!("invalid web path: {error}\n"),
                request.method,
            )?;
            return Ok(());
        }
    };
    let path = gateway.web_root.join(relative);
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            write_text(stream, 404, "Not Found", "file not found\n", request.method)?;
            return Ok(());
        }
        Err(error) => return Err(error).with_context(|| format!("opening {}", path.display())),
    };
    let length = file.metadata()?.len();
    let content_type = content_type(&path);
    write_head(
        stream,
        200,
        "OK",
        content_type,
        length,
        &[("Cache-Control", "no-cache")],
    )?;
    if request.method == Method::Get {
        std::io::copy(&mut file, stream)?;
    }
    Ok(())
}

fn safe_static_path(target: &str) -> Result<PathBuf> {
    let decoded = percent_decode(target.trim_start_matches('/'))?;
    let decoded = String::from_utf8(decoded).context("web path is not UTF-8")?;
    let mut path = PathBuf::new();
    for component in decoded.split('/') {
        if component.is_empty() || matches!(component, "." | "..") || component.contains('\\') {
            bail!("empty or traversal component");
        }
        path.push(component);
    }
    Ok(path)
}

fn percent_decode(raw: &str) -> Result<Vec<u8>> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                bail!("truncated percent escape");
            }
            let high = hex(bytes[index + 1]).context("invalid percent escape")?;
            let low = hex(bytes[index + 2]).context("invalid percent escape")?;
            out.push((high << 4) | low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    Ok(out)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn write_text(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    text: &str,
    method: Method,
) -> Result<()> {
    write_bytes(
        stream,
        status,
        reason,
        "text/plain; charset=utf-8",
        text.as_bytes(),
        &[("Cache-Control", "no-store")],
        method,
    )
}

fn write_bytes(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    bytes: &[u8],
    headers: &[(&str, &str)],
    method: Method,
) -> Result<()> {
    write_head(
        stream,
        status,
        reason,
        content_type,
        bytes.len() as u64,
        headers,
    )?;
    if method == Method::Get && status != 304 {
        stream.write_all(bytes)?;
    }
    Ok(())
}

fn write_head(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    content_length: u64,
    headers: &[(&str, &str)],
) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {content_length}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nAccess-Control-Allow-Origin: *\r\n"
    )?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    Ok(())
}
