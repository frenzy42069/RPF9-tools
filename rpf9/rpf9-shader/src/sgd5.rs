//! SGD5 shader blobs from `shaders.rpf` — decompilation of the visible
//! structure: the blob hash, the entry count, the header words, and the
//! entry table.
//!
//! Observed layout (verified on the local `shaders.rpf`):
//!
//! ```text
//! +0x00  magic "SGD5"
//! +0x04  u64 blob hash
//! +0x0C  u32 entry count
//! +0x10  header words (u32), dumped while nonzero
//! +0x30  entry table: pairs of (u32 tag, u32 value), `count` entries
//! ```
//!
//! Fields beyond the hash/count/entry table are raw dumps — the entry-table
//! semantics are not decoded yet.

use std::fmt::Write as _;

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    data.get(at..at + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
}

fn u64_at(data: &[u8], at: usize) -> Option<u64> {
    data.get(at..at + 8)
        .and_then(|b| b.try_into().ok())
        .map(u64::from_le_bytes)
}

#[derive(Debug, Default, Clone)]
pub struct Sgd5 {
    pub hash: u64,
    pub count: u32,
    /// Header words from +0x10 while nonzero: (offset, value).
    pub fields: Vec<(usize, u32)>,
    /// Entry table records from +0x30: (offset, tag, value).
    pub entries: Vec<(usize, u32, u32)>,
}

impl Sgd5 {
    pub fn parse(data: &[u8]) -> Option<Sgd5> {
        if !data.starts_with(b"SGD5") {
            return None;
        }
        let mut s = Sgd5 {
            hash: u64_at(data, 0x04).unwrap_or(0),
            count: u32_at(data, 0x0C).unwrap_or(0),
            ..Sgd5::default()
        };
        let mut o = 0x10;
        while let Some(v) = u32_at(data, o) {
            if v == 0 || s.fields.len() >= 8 {
                break;
            }
            s.fields.push((o, v));
            o += 4;
        }
        let want = if s.count > 0 { s.count as usize } else { 4096 };
        let mut o = 0x30;
        for _ in 0..want {
            let (Some(tag), Some(value)) = (u32_at(data, o), u32_at(data, o + 4)) else {
                break;
            };
            s.entries.push((o, tag, value));
            o += 8;
        }
        Some(s)
    }
}

pub fn decompile(path: &std::path::Path, data: &[u8]) -> String {
    let s = Sgd5::parse(data).expect("magic checked by caller");
    let mut out = String::new();
    writeln!(out, "== SGD5 blob ==").unwrap();
    writeln!(out, "path: {}", path.display()).unwrap();
    writeln!(out, "magic: SGD5").unwrap();
    writeln!(out, "hash: {:#018x} (u64 @ +0x04)", s.hash).unwrap();
    writeln!(out, "count: {} (u32 @ +0x0C)", s.count).unwrap();
    writeln!(out, "header words:").unwrap();
    for (o, v) in &s.fields {
        writeln!(out, "  +{o:#04x}: {v:#010x}").unwrap();
    }
    writeln!(out, "entries (tag, value):").unwrap();
    for (o, tag, value) in &s.entries {
        writeln!(out, "  +{o:#06x}: {tag:#010x} = {value:#010x}").unwrap();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synthetic_blob() {
        let mut d = b"SGD5".to_vec();
        d.extend_from_slice(&0x1122334455667788u64.to_le_bytes());
        d.extend_from_slice(&3u32.to_le_bytes()); // count
        d.extend_from_slice(&0x840u32.to_le_bytes()); // header word 1
        d.extend_from_slice(&0u32.to_le_bytes()); // stop
        d.extend_from_slice(&[0; 0x18]); // pad to +0x30
        d.extend_from_slice(&0x0405u32.to_le_bytes()); // entry 1 tag
        d.extend_from_slice(&0x3b316b01u32.to_le_bytes()); // entry 1 value
        d.extend_from_slice(&0x10405u32.to_le_bytes()); // entry 2 tag
        d.extend_from_slice(&0xd35e47edu32.to_le_bytes()); // entry 2 value

        let s = Sgd5::parse(&d).unwrap();
        assert_eq!(s.hash, 0x1122334455667788);
        assert_eq!(s.count, 3);
        assert_eq!(s.fields, vec![(0x10, 0x840)]);
        assert_eq!(s.entries.len(), 2);
        assert_eq!(s.entries[0], (0x30, 0x0405, 0x3b316b01));
        assert_eq!(s.entries[1], (0x38, 0x10405, 0xd35e47ed));
    }

    #[test]
    fn parses_real_blob() {
        let Ok(d) = std::fs::read("../results/shaders/sgd5_a.bin") else {
            return; // results/ not extracted yet.
        };
        let s = Sgd5::parse(&d).expect("parse");
        assert_eq!(s.hash, 0x60555dc2a08b72b3);
        assert_eq!(s.count, 33);
        assert_eq!(s.entries.len(), 33);
        assert_eq!(s.entries[0].1, 0x00000405);
    }
}
