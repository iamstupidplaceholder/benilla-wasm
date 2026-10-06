//! Exact file-presence metadata; no listfile enumeration, filesystem or decompression.

use crate::{find_block_index, BlockEntry, Error, HashEntry, Result, FLAG_DELETE_MARKER};
use benilla_bytes::ByteExt;

#[derive(Clone)]
pub struct FilePresenceIndex {
    hashes: Vec<HashEntry>,
    flags: Vec<u32>,
}

impl FilePresenceIndex {
    pub fn encoded_len(&self) -> usize {
        16usize
            .saturating_add(self.hashes.len().saturating_mul(12))
            .saturating_add(self.flags.len().saturating_mul(4))
    }
    pub(crate) fn from_tables(hashes: &[HashEntry], blocks: &[BlockEntry]) -> Self {
        Self {
            hashes: hashes.to_vec(),
            flags: blocks.iter().map(|b| b.flags).collect(),
        }
    }

    /// None: no entry; Some(false): tombstone; Some(true): file.
    /// This preserves Archive's lookup verdict (not byte/decode success).
    pub fn lookup(&self, name: &str) -> Option<bool> {
        self.flags
            .get(find_block_index(&self.hashes, name)?)
            .map(|flags| flags & FLAG_DELETE_MARKER == 0)
    }

    /// Versioned little-endian metadata: magic, counts, hash triples, block flags.
    /// Carries neither archive filesystem paths nor file payloads.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = b"BPFI\x01\x00\x00\x00".to_vec();
        bytes.extend_from_slice(&(self.hashes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(self.flags.len() as u32).to_le_bytes());
        for hash in &self.hashes {
            for word in [hash.name_a, hash.name_b, hash.block_index] {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
        }
        for flags in &self.flags {
            bytes.extend_from_slice(&flags.to_le_bytes());
        }
        bytes
    }

    /// Validate total span before any count-controlled allocation.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let bad = || Error::Corrupt("invalid file-presence index".into());
        if bytes.get(..8) != Some(b"BPFI\x01\x00\x00\x00".as_slice()) {
            return Err(bad());
        }
        let hashes = bytes.u32_at(8).ok_or_else(bad)? as usize;
        let flags = bytes.u32_at(12).ok_or_else(bad)? as usize;
        let flags_at = hashes
            .checked_mul(12)
            .and_then(|n| n.checked_add(16))
            .ok_or_else(bad)?;
        let end = flags
            .checked_mul(4)
            .and_then(|n| n.checked_add(flags_at))
            .ok_or_else(bad)?;
        if end != bytes.len() {
            return Err(bad());
        }
        let mut hash_table = Vec::with_capacity(hashes);
        for i in 0..hashes {
            let at = 16 + i * 12;
            hash_table.push(HashEntry {
                name_a: bytes.u32_at(at).ok_or_else(bad)?,
                name_b: bytes.u32_at(at + 4).ok_or_else(bad)?,
                block_index: bytes.u32_at(at + 8).ok_or_else(bad)?,
            });
        }
        let mut block_flags = Vec::with_capacity(flags);
        for i in 0..flags {
            block_flags.push(bytes.u32_at(flags_at + i * 4).ok_or_else(bad)?);
        }
        Ok(Self {
            hashes: hash_table,
            flags: block_flags,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_counts_versions_and_trailing_bytes_fail_before_allocation() {
        let index = FilePresenceIndex::from_tables(&[], &[]);
        let valid = index.encode();
        assert_eq!(
            FilePresenceIndex::decode(&valid).unwrap().lookup("absent"),
            None
        );
        for len in 0..valid.len() {
            assert!(FilePresenceIndex::decode(&valid[..len]).is_err());
        }
        let mut bad = valid.clone();
        bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(FilePresenceIndex::decode(&bad).is_err());
        let mut bad = valid.clone();
        bad[4] = 2;
        assert!(FilePresenceIndex::decode(&bad).is_err());
        let mut bad = valid;
        bad.push(0);
        assert!(FilePresenceIndex::decode(&bad).is_err());
    }
}
