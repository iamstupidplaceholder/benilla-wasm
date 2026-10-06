//! Exact MPQ metadata delivery, separate from logical file reads and game relays.

use super::*;

pub(crate) const ROUTE: &str = "/assets/v1/presence.bfi";

pub(crate) fn build(chain: &Chain, max_bytes: u64) -> Result<CachedAsset> {
    let index = chain.presence_index();
    let size = index.encoded_len();
    if size as u64 > max_bytes || size > u32::MAX as usize {
        bail!("presence index exceeds gateway metadata limit ({size} bytes)");
    }
    let bytes = index.encode();
    Ok(CachedAsset {
        etag: format!("\"{:x}\"", Sha1::digest(&bytes)),
        bytes: Arc::new(bytes),
        archive: "presence-index".into(),
    })
}

pub(crate) fn serve(asset: &CachedAsset, stream: &mut TcpStream, request: &Request) -> Result<()> {
    let headers = [
        ("ETag", asset.etag.as_str()),
        ("Cache-Control", "public, max-age=0, must-revalidate"),
    ];
    if request.if_none_match.as_deref() == Some(asset.etag.as_str()) {
        write_bytes(
            stream,
            304,
            "Not Modified",
            "application/octet-stream",
            &[],
            &headers,
            request.method,
        )
    } else {
        write_bytes(
            stream,
            200,
            "OK",
            "application/octet-stream",
            &asset.bytes,
            &headers,
            request.method,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(method: Method, conditional: bool) -> Vec<u8> {
        let index =
            benilla_formats::ChainPresenceIndex::decode(b"BCPI\x01\x00\x00\x00\x00\x00\x00\x00")
                .unwrap();
        let bytes = index.encode();
        assert_eq!(index.encoded_len(), bytes.len());
        let asset = CachedAsset {
            etag: format!("\"{:x}\"", Sha1::digest(&bytes)),
            bytes: Arc::new(bytes),
            archive: "not-an-exposed-host-path".into(),
        };
        let request = Request {
            method,
            target: ROUTE.into(),
            if_none_match: conditional.then(|| asset.etag.clone()),
            host: None,
            origin: None,
            upgrade: None,
            connection: None,
            websocket_key: None,
            websocket_version: None,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let worker = std::thread::spawn(move || {
            let (mut server, _) = listener.accept().unwrap();
            serve(&asset, &mut server, &request).unwrap();
        });
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        worker.join().unwrap();
        response
    }

    #[test]
    fn metadata_get_head_and_conditional_responses_preserve_the_binary_contract() {
        let get = response(Method::Get, false);
        let split = get.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let headers = std::str::from_utf8(&get[..split]).unwrap();
        assert!(headers.starts_with("HTTP/1.1 200 OK"));
        assert!(headers.contains("Content-Length: 12"));
        assert!(headers.contains("ETag:"));
        assert!(!headers.contains("X-Benilla-Archive"));
        assert!(benilla_formats::ChainPresenceIndex::decode(&get[split..]).is_ok());
        let head = String::from_utf8(response(Method::Head, false)).unwrap();
        assert!(head.contains("Content-Length: 12"));
        assert!(head.ends_with("\r\n\r\n"));
        let conditional = String::from_utf8(response(Method::Get, true)).unwrap();
        assert!(conditional.starts_with("HTTP/1.1 304 Not Modified"));
        assert!(conditional.ends_with("\r\n\r\n"));
    }
}
