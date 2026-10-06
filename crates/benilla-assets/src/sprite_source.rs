//! Production sprite resolution/decoding, independent of host byte delivery.
//! Pending is not a miss: neither cache it nor choose a lower-priority candidate.

use benilla_formats::{blp_to_rgba, tga_to_rgba};

mod asset_server;
pub(crate) mod native;
pub use asset_server::{SpriteFileReader, SpriteFiles};

/// A host's answer for one exact file path. Delivery failures become Missing
/// only after the host has reported its IO error; undecodable bytes stay Bytes.
pub enum SpriteRead {
    Pending,
    Missing,
    Bytes(Vec<u8>),
}

/// The original two-store read order, for every candidate: installed archive,
/// then loose AddOns. Hosts without a loose store use the default second answer.
pub trait SpriteSource {
    fn store_description(&self) -> &str {
        "host byte source"
    }
    fn read_installed(&mut self, path: &str) -> SpriteRead;
    fn read_loose(&mut self, _path: &str) -> SpriteRead {
        SpriteRead::Missing
    }
}

#[derive(Debug, PartialEq)]
pub enum SpriteDecode {
    Ready(u32, u32, Vec<u8>),
    Pending,
    /// Candidates that were present but refused by the original decoders.
    /// Empty means no candidate was present in either store.
    Failed {
        undecodable: Vec<String>,
    },
}

/// Same candidate order and content-based BLP/TGA dispatch as WorldAssets.
/// A bad candidate falls through; an unfinished candidate does not.
pub fn decode_sprite_from_source(source: &mut impl SpriteSource, path: &str) -> SpriteDecode {
    let candidates = crate::sprite_candidates(path);
    let mut undecodable = Vec::new();
    for candidate in &candidates {
        for loose in [false, true] {
            let read = if loose {
                source.read_loose(candidate)
            } else {
                source.read_installed(candidate)
            };
            match read {
                SpriteRead::Pending => return SpriteDecode::Pending,
                SpriteRead::Missing => {}
                SpriteRead::Bytes(bytes) => {
                    if let Ok((w, h, rgba)) = decode_sprite_bytes(&bytes) {
                        return SpriteDecode::Ready(w, h, rgba);
                    }
                    if !undecodable.contains(candidate) {
                        undecodable.push(candidate.clone());
                    }
                }
            }
        }
    }
    SpriteDecode::Failed { undecodable }
}

/// Content, not extension: real addon folders ship mislabeled BLP and TGA files.
fn decode_sprite_bytes(bytes: &[u8]) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    if bytes.starts_with(b"BLP2") {
        blp_to_rgba(bytes)
    } else {
        tga_to_rgba(bytes)
    }
}

/// Outer None means pending (do not insert); inner None is a completed miss.
pub(crate) fn decode_completed(
    source: &mut impl SpriteSource,
    path: &str,
) -> Option<Option<(u32, u32, Vec<u8>)>> {
    match decode_sprite_from_source(source, path) {
        SpriteDecode::Ready(w, h, rgba) => Some(Some((w, h, rgba))),
        SpriteDecode::Pending => None,
        SpriteDecode::Failed { undecodable } => {
            if undecodable.is_empty() {
                bevy::log::warn!(
                    "texture miss: '{path}' does not resolve in the {} (tried {})",
                    source.store_description(),
                    crate::sprite_candidates(path).join(", ")
                );
            } else {
                bevy::log::warn!("texture miss: '{path}' RESOLVES but will not decode ({}) — the file is there and the decoder refused it", undecodable.join(", "));
            }
            Some(None)
        }
    }
}

#[cfg(test)]
mod tests;
