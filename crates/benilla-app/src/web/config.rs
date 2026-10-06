//! Deployment endpoints resolved by full.html from editable web/config.js.
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
export function hideBootstrapStatus() {
    const status = document.getElementById('benilla-bootstrap-status');
    if (status) status.hidden = true;
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = hideBootstrapStatus)]
    pub(crate) fn hide_bootstrap_status();
}

pub(crate) fn active_locale() -> String {
    endpoint("locale")
}
/// Before the original UI VM exists, use the page's own en/ru startup dictionary.
pub(crate) fn startup_text(key: &str) -> String {
    let dictionary = js_sys::Reflect::get(&js_sys::global(), &"BENILLA_I18N".into())
        .expect("reading startup localization");
    let function = js_sys::Reflect::get(&dictionary, &"text".into())
        .expect("reading startup localization text");
    use wasm_bindgen::JsCast;
    function
        .unchecked_into::<js_sys::Function>()
        .call1(&dictionary, &key.into())
        .expect("formatting startup localization")
        .as_string()
        .expect("startup localization returns text")
}
/// Browser-owned chrome has no original GlobalStrings keys. Keep its bilingual
/// wording separate from installed game text and select before the VM exists.
pub(crate) fn text<'a>(english: &'a str, russian: &'a str) -> &'a str {
    if active_locale() == "ruRU" {
        russian
    } else {
        english
    }
}
pub(crate) fn endpoint(key: &str) -> String {
    let endpoints = js_sys::Reflect::get(&js_sys::global(), &"BENILLA_ENDPOINTS".into())
        .expect("reading BENILLA_ENDPOINTS");
    js_sys::Reflect::get(&endpoints, &key.into())
        .expect("reading browser gateway endpoint")
        .as_string()
        .unwrap_or_else(|| {
            panic!("missing BENILLA_ENDPOINTS.{key}; load web/config.js through full.html")
        })
}
