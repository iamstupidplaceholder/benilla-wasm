//! Exact patch-chain presence metadata for synchronous consumers without archive IO.

use anyhow::{bail, Context, Result};
use benilla_bytes::ByteExt;
use benilla_mpq::FilePresenceIndex;

/// Higher-priority entries are visited first. A deletion is a verdict, not a
/// reason to fall through to the lower archive's readable copy.
pub(crate) fn readable_winner(mut verdicts: impl Iterator<Item = Option<bool>>) -> bool {
    verdicts.find_map(|verdict| verdict).unwrap_or(false)
}

#[derive(Clone)]
pub struct ChainPresenceIndex {
    /// Same low-to-high precedence order as Chain::archives.
    pub(crate) archives: Vec<FilePresenceIndex>,
}

impl ChainPresenceIndex {
    pub fn encoded_len(&self) -> usize {
        self.archives.iter().fold(12usize, |size, index| {
            size.saturating_add(4).saturating_add(index.encoded_len())
        })
    }
    pub fn contains(&self, name: &str) -> bool {
        readable_winner(self.archives.iter().rev().map(|index| index.lookup(name)))
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = b"BCPI\x01\x00\x00\x00".to_vec();
        bytes.extend_from_slice(&(self.archives.len() as u32).to_le_bytes());
        for index in &self.archives {
            let encoded = index.encode();
            bytes.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&encoded);
        }
        bytes
    }

    /// Count-controlled allocations are bounded by the delivered payload span.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.get(..8) != Some(b"BCPI\x01\x00\x00\x00".as_slice()) {
            bail!("invalid chain-presence index version");
        }
        let count = bytes.u32_at(8).context("missing chain-presence count")? as usize;
        if count > bytes.len().saturating_sub(12) / 4 {
            bail!("chain-presence count exceeds payload");
        }
        let mut archives = Vec::with_capacity(count);
        let mut at = 12usize;
        for _ in 0..count {
            let len = bytes.u32_at(at).context("missing archive-index length")? as usize;
            at = at.checked_add(4).context("archive-index offset overflow")?;
            let end = at
                .checked_add(len)
                .context("archive-index length overflow")?;
            let payload = bytes
                .get(at..end)
                .context("truncated archive-index payload")?;
            archives.push(FilePresenceIndex::decode(payload)?);
            at = end;
        }
        if at != bytes.len() {
            bail!("trailing chain-presence bytes");
        }
        Ok(Self { archives })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tombstones_shadow_lower_files_and_later_files_restore_them() {
        assert!(!readable_winner(
            [None, Some(false), Some(true)].into_iter()
        ));
        assert!(readable_winner(
            [Some(true), Some(false), Some(true)].into_iter()
        ));
        assert!(readable_winner([None, None, Some(true)].into_iter()));
        assert!(!readable_winner([None, None].into_iter()));
    }

    #[test]
    fn chain_metadata_rejects_truncation_counts_and_trailing_bytes() {
        let encoded = ChainPresenceIndex { archives: vec![] }.encode();
        assert!(!ChainPresenceIndex::decode(&encoded)
            .unwrap()
            .contains("absent"));
        for len in 0..encoded.len() {
            assert!(ChainPresenceIndex::decode(&encoded[..len]).is_err());
        }
        let mut bad = encoded.clone();
        bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(ChainPresenceIndex::decode(&bad).is_err());
        let mut bad = encoded;
        bad.push(0);
        assert!(ChainPresenceIndex::decode(&bad).is_err());
    }
}
