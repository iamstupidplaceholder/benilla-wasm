//! Platform-neutral primitives used directly by the production FrameScript host.
//!
//! Behavior stays in `script/object.rs`, `script/region.rs` and `script/event.rs`.
//! This module contains only value vocabulary that is identical on every Lua
//! backend and is already consumed by those original modules.

use std::borrow::Cow;

pub(crate) const REG_FRAME_META: &str = "__benilla_frame_meta";
pub(crate) const REG_REGION_META: &str = "__benilla_region_meta";
pub(crate) const REG_FRAME_METHODS: &str = "__benilla_frame_methods";
pub(crate) const REG_REGION_METHODS: &str = "__benilla_region_methods";
pub(crate) const REG_TITLE_METHODS: &str = "__benilla_title_methods";
pub(crate) const REG_TITLE_META: &str = "__benilla_title_meta";
pub(crate) const REG_TEXTURE_METHODS: &str = "__benilla_texture_methods";
pub(crate) const REG_TEXTURE_META: &str = "__benilla_texture_meta";
pub(crate) const REG_FONTSTRING_METHODS: &str = "__benilla_fontstring_methods";
pub(crate) const REG_FONTSTRING_META: &str = "__benilla_fontstring_meta";
pub(crate) const REG_DEFAULT_ERRORHANDLER: &str = "__benilla_default_errorhandler";
pub(crate) const REG_WRAPPERS: &str = "__benilla_wrappers";
pub(crate) const REG_SCRIPTS: &str = "__benilla_scripts";

/// Encode the production object's stable id as its light-userdata payload.
pub(crate) fn object_id_bits(id: u32) -> usize {
    id as usize
}

/// Validate and decode the light-userdata payload stored in wrapper slot zero.
pub(crate) fn object_id_from_bits(bits: usize) -> Option<u32> {
    u32::try_from(bits).ok()
}

const ARG_NAMES: [&str; 16] = [
    "arg1", "arg2", "arg3", "arg4", "arg5", "arg6", "arg7", "arg8", "arg9", "arg10", "arg11",
    "arg12", "arg13", "arg14", "arg15", "arg16",
];

/// The legacy global name for a 1-based handler argument.
pub(crate) fn arg_name(index: usize) -> Cow<'static, str> {
    match ARG_NAMES.get(index - 1) {
        Some(&name) => Cow::Borrowed(name),
        None => Cow::Owned(format!("arg{index}")),
    }
}
