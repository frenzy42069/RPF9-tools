//! Xbox Series X PSO cache (`PS\x00\x09`) from `shaders.rpf` — decompilation
//! of the visible structure: cache id, version, hash, and the header words.
//!
//! Observed layout (verified on the local `shaders.rpf`):
//!
//! ```text
//! +0x00  magic "PS\0\x09"
//! +0x04  id string, NUL-terminated (e.g. "0x65F41EDD")
//! +0x24  version string, NUL-terminated (e.g. "930.048.020")
//! +0x48  u64 hash
//! +0x50  header words (u32), dumped while nonzero
//! ```
//!
//! Program entries in the cache body are not decoded — only the visible
//! header is.

use std::fmt::Write as _;

fn cstring(data: &[u8], at: usize) -> Option<String> {
    let end = data.get(at..)?.iter().position(|&b| b == 0)? + at;
    String::from_utf8_lossy(&data[at..end]).into_owned().into()
}

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
pub struct PsoCache {
    pub id: String,
    pub version: String,
    pub hash: Option<u64>,
    pub fields: Vec<(usize, u32)>,
}

impl PsoCache {
    pub fn parse(data: &[u8]) -> Option<PsoCache> {
        if !data.starts_with(b"PS\x00\x09") {
            return None;
        }
        let mut c = PsoCache {
            id: cstring(data, 0x04).unwrap_or_default(),
            version: cstring(data, 0x24).unwrap_or_default(),
            hash: u64_at(data, 0x48),
            ..PsoCache::default()
        };
        let mut o = 0x50;
        while let Some(v) = u32_at(data, o) {
            if v == 0 || c.fields.len() >= 16 {
                break;
            }
            c.fields.push((o, v));
            o += 4;
        }
        Some(c)
    }
}

pub fn decompile(path: &std::path::Path, data: &[u8]) -> String {
    let c = PsoCache::parse(data).expect("magic checked by caller");
    let mut out = String::new();
    writeln!(out, "== PSO cache ==").unwrap();
    writeln!(out, "path: {}", path.display()).unwrap();
    writeln!(out, "magic: PS\\x00\\x09").unwrap();
    writeln!(out, "id: {}", c.id).unwrap();
    writeln!(out, "version: {}", c.version).unwrap();
    if let Some(h) = c.hash {
        writeln!(out, "hash: {h:#018x} (u64 @ +0x48)").unwrap();
    }
    writeln!(out, "header words:").unwrap();
    for (o, v) in &c.fields {
        writeln!(out, "  +{o:#04x}: {v:#010x}").unwrap();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synthetic_cache() {
        let mut d = b"PS\x00\x09".to_vec();
        d.extend_from_slice(b"0xDEADBEEF\0"); // id @ +0x04
        d.resize(0x24, 0);
        d.extend_from_slice(b"930.048.020\0"); // version @ +0x24
        d.resize(0x48, 0);
        d.extend_from_slice(&0x07efb73d13c391cfu64.to_le_bytes()); // hash @ +0x48
        d.extend_from_slice(&0x637fu32.to_le_bytes()); // field @ +0x50
        d.extend_from_slice(&0xa85u32.to_le_bytes()); // field @ +0x54

        let c = PsoCache::parse(&d).unwrap();
        assert_eq!(c.id, "0xDEADBEEF");
        assert_eq!(c.version, "930.048.020");
        assert_eq!(c.hash, Some(0x07efb73d13c391cf));
        assert_eq!(c.fields, vec![(0x50, 0x637f), (0x54, 0xa85)]);
    }

    #[test]
    fn parses_real_cache() {
        let Ok(d) = std::fs::read("../results/shaders/h000000/10579000.ps09") else {
            return; // results/ not extracted yet.
        };
        let c = PsoCache::parse(&d).expect("parse");
        assert_eq!(c.id, "0x65F41EDD");
        assert_eq!(c.version, "930.048.020");
        assert_eq!(c.hash, Some(0x07efb73d13c391cf));
        assert!(!c.fields.is_empty());
    }
}
