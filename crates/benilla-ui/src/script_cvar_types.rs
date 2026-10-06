//! Engine-side CVar and display-capability values shared by native and browser UI hosts.

/// One applied CVar row copied from the host registry into the Lua mirror.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeededCvar {
    pub name: String,
    pub value: String,
    pub default: String,
    pub latched: bool,
}

/// A device-supported multisample mode; one sample means no multisampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MultisampleFormat {
    pub color_bits: u32,
    pub depth_bits: u32,
    pub samples: u32,
}

/// A display mode in physical pixels, formatted as the client's `WxH` string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenResolution {
    pub width: u32,
    pub height: u32,
}

impl ScreenResolution {
    /// The client sorts by pixel area; width and height break equal-area ties.
    pub(crate) fn sort_key(&self) -> (u64, u32, u32) {
        (
            u64::from(self.width) * u64::from(self.height),
            self.width,
            self.height,
        )
    }
}

impl std::fmt::Display for ScreenResolution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

/// The seven `GetVideoCaps` answers supplied by the rendering host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct VideoCaps {
    pub anisotropic: bool,
    pub pixel_shaders: bool,
    pub vertex_shaders: bool,
    pub trilinear: bool,
    pub triple_buffering: bool,
    pub max_anisotropy: u32,
    pub hardware_cursor: bool,
}
