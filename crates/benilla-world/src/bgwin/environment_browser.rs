//! A browser tab has no process environment or native background-run flags.
//! Do not call std's unsupported environment enumeration on wasm.

pub(super) fn names() -> impl Iterator<Item = std::ffi::OsString> {
    std::iter::empty()
}
