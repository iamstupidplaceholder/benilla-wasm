#[path = "../../../entities/attach/char_skin.rs"]
pub(super) mod char_skin;
#[path = "../../../entities/attach/preview.rs"]
mod preview;

pub(super) use preview::{build_glue_pet, build_glue_preview, glue_preview_spec};
