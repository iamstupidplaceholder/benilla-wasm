//! HTTP delivery shared by MPQ files and gateway metadata. Before world entry,
//! recoverable transport/body faults retry indefinitely with bounded attempts
//! and backoff. In-world delivery stays bounded. Definitive HTTP errors and
//! decoder failures remain terminal; no fabricated successful resource bytes.
use bevy::asset::io::AssetReaderError;
use std::{path::Path, sync::Arc};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(inline_js = r#"
function bounded(value, fallback, min, max) {
    return typeof value === 'number' && Number.isFinite(value)
        ? Math.max(min, Math.min(max, Math.floor(value))) : fallback;
}
export async function deliverAsset(url) {
    const text = (key, values) => globalThis.BENILLA_I18N.text(key, values);
    const active = globalThis.BENILLA_ASSET_DELIVERIES ||= new Map();
    const entry = { url, attempt: 0, error: '', started: performance.now() };
    active.set(entry, entry);
    try {
    const config = globalThis.BENILLA_CONFIG?.assetDelivery || {};
    const attempts = bounded(config.attempts, 3, 1, 8);
    const attemptTimeout = bounded(config.attemptTimeoutMs, 10000, 100, 120000);
    const totalTimeout = bounded(config.totalTimeoutMs, 35000, 100, 180000);
    const retryDelay = bounded(config.retryDelayMs, 250, 0, 10000);
    const deadline = performance.now() + totalTimeout;
    let result = { status: 0, error: text('asset_budget_exhausted') };
    for (let attempt = 0; ; ++attempt) {
        const persistent = globalThis.BENILLA_ASSET_PREWORLD !== false;
        const remaining = persistent ? attemptTimeout : deadline - performance.now();
        if (remaining <= 0) break;
        entry.attempt = attempt + 1;
        const controller = new AbortController();
        let timer;
        try {
            const timeout = new Promise((_, reject) => {
                timer = setTimeout(() => {
                    controller.abort();
                    reject(new Error(text('asset_timeout')));
                }, Math.min(attemptTimeout, remaining));
            });
            result = await Promise.race([timeout, (async () => {
                const response = await fetch(url, { signal: controller.signal });
                if (response.status !== 200) {
                    if (response.body) await response.body.cancel();
                    return { status: response.status, error: `HTTP ${response.status}` };
                }
                const bytes = new Uint8Array(await response.arrayBuffer());
                // Empty Lua/TOC files can be authored; binary assets and metadata cannot.
                if (!bytes.length && !/\.(lua|toc)(?:$|\?)/i.test(url)) throw new Error(text('asset_empty'));
                return { status: 200, bytes };
            })()]);
        } catch (error) {
            result = { status: 0, error: error instanceof TypeError
                ? text('asset_network_error') : (error.message || String(error)) };
        }
        finally { clearTimeout(timer); }
        if (result.status === 200) return result;
        const retryable = result.status === 0 || result.status === 408 || result.status === 429
            || result.status >= 500;
        entry.error = result.error;
        if (!retryable || (!persistent && attempt + 1 >= attempts)) break;
        const delay = Math.min(Math.max(100, retryDelay) * (2 ** Math.min(attempt, 6)), 10000,
            persistent ? 10000 : Math.max(0, deadline - performance.now()));
        if (delay) await new Promise(resolve => setTimeout(resolve, delay));
    }
    result.error = `${url}: ${result.error} (${text('asset_terminal')})`;
    return result;
    } finally { active.delete(entry); }
}
export function deliveryContext(preworld, stage) {
    globalThis.BENILLA_ASSET_PREWORLD = preworld;
    let panel = document.getElementById('benilla-delivery-status');
    if (!panel) {
        panel = document.createElement('div'); panel.id = 'benilla-delivery-status';
        panel.style.cssText = 'position:fixed;bottom:12px;left:12px;right:12px;z-index:9999;pointer-events:none;color:#ffd778;background:#10131cdd;padding:8px;font:14px sans-serif;overflow-wrap:anywhere';
        document.body.appendChild(panel);
    }
    const requests = Array.from(globalThis.BENILLA_ASSET_DELIVERIES?.values() || []);
    requests.sort((a,b)=>a.started-b.started);
    const waiting = requests[0];
    panel.hidden = !stage;
    const text = (key, values) => globalThis.BENILLA_I18N.text(key, values);
    panel.textContent = stage + (waiting ? ` · ${text('pending_files', { count: requests.length })}\n${waiting.url} · ${text('attempt', { count: waiting.attempt })}${waiting.error ? ' · '+waiting.error : ''}` : '');
}
export function deliveryPending() { return (globalThis.BENILLA_ASSET_DELIVERIES?.size || 0) > 0; }
"#)]
extern "C" {
    #[wasm_bindgen(js_name = deliveryContext)]
    pub fn delivery_context(preworld: bool, stage: &str);
    #[wasm_bindgen(js_name = deliveryPending)]
    pub fn delivery_pending() -> bool;
    #[wasm_bindgen(js_name = deliverAsset)]
    fn deliver_asset(url: &str) -> js_sys::Promise;
}

pub(crate) async fn read(endpoint: &str, path: &Path) -> Result<Vec<u8>, AssetReaderError> {
    let url = format!(
        "{}/{}",
        endpoint.trim_end_matches('/'),
        path.to_string_lossy()
    );
    let result = JsFuture::from(deliver_asset(&url)).await.map_err(|error| {
        AssetReaderError::Io(Arc::new(std::io::Error::other(format!("{url}: {error:?}"))))
    })?;
    let status = js_sys::Reflect::get(&result, &"status".into())
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as u16;
    match status {
        200 => {
            let bytes = js_sys::Reflect::get(&result, &"bytes".into()).map_err(|error| {
                AssetReaderError::Io(Arc::new(std::io::Error::other(format!("{url}: {error:?}"))))
            })?;
            Ok(js_sys::Uint8Array::new(&bytes).to_vec())
        }
        404 => Err(AssetReaderError::NotFound(path.to_path_buf())),
        0 => {
            let error = js_sys::Reflect::get(&result, &"error".into())
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_else(|| format!("{url}: delivery failed"));
            Err(AssetReaderError::Io(Arc::new(std::io::Error::other(error))))
        }
        status => Err(AssetReaderError::HttpError(status)),
    }
}
