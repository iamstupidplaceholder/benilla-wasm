//! Native launch-instrument discovery reads the process environment.

pub(super) fn names() -> impl Iterator<Item = std::ffi::OsString> {
    std::env::vars_os().map(|(name, _)| name)
}
