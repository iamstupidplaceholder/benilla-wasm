//! A backend-neutral view of one Lua call-stack frame, for `debugstack`.
//!
//! The production formatter (`benilla_ui::script::stdlib`) renders these
//! into the reference's own 5.0 frame wording; both backends supply the same
//! raw fields. Level 0 is the innermost running function.

/// What kind of function a stack frame runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugWhat {
    /// The outermost chunk body.
    Main,
    /// An ordinary Lua function.
    Lua,
    /// A host (Rust/C) function.
    C,
}

/// One frame of [`crate::LuaContext::debug_frames`].
#[derive(Clone, Debug)]
pub struct DebugFrame {
    /// The chunk name as compiled (`@path`, `=name`, or raw source text).
    /// `None` for host frames, which carry no chunk.
    pub source: Option<String>,
    /// The printable source form. `None` is rendered as `"?"`.
    pub short_src: Option<String>,
    /// The line where the chunk's function was defined, if known.
    pub line_defined: Option<usize>,
    /// The line currently executing, if known.
    pub current_line: Option<usize>,
    /// The function's name, if the caller names it.
    pub name: Option<String>,
    /// The frame kind.
    pub what: DebugWhat,
}
