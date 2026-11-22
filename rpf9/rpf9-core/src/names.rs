//! Embedded names in decoded resource headers.

use std::collections::HashSet;

use crate::bytes::le64;
use crate::kinds::Kind;
use crate::stream::Stream;

/// Read the resource name from the system page of a known drawable layout.
pub fn drawable_name(page: &[u8]) -> Option<&str> {
    let pointer = le64(page, 0xa8)?;
    let offset = pointer.checked_sub(0x5000_0000)?;
    if offset >= 0x1000_0000 {
        return None;
    }
    let bytes = page.get(offset as usize..)?;
    let end = bytes.iter().position(|&byte| byte == 0)?;
    let name = std::str::from_utf8(&bytes[..end]).ok()?;
    let stem = name.strip_suffix(".#dr")?;
    (!stem.is_empty()).then_some(name)
}

/// Recover a portable filename stem from a decoded stream when its layout is known.
pub fn embedded_stem(stream: &Stream, page: &[u8]) -> Option<String> {
    if !matches!(stream.kind, Kind::Resource | Kind::Rsc9) {
        return None;
    }
    drawable_name(page).and_then(portable_stem)
}

/// Assign consistent filenames within a single archive directory.
#[derive(Default)]
pub struct Filenames {
    used: HashSet<String>,
}

impl Filenames {
    pub fn for_stream(&mut self, stream: &Stream, stem: Option<&str>) -> String {
        self.for_entry(stream.at, stream.kind.label().0, stem)
    }

    /// Assign a filename from decoded or cached entry metadata.
    pub fn for_entry(&mut self, offset: usize, ext: &str, stem: Option<&str>) -> String {
        match stem {
            Some(stem) => unique_name(stem, ext, offset, &mut self.used),
            None => format!("{offset:08x}.{ext}"),
        }
    }
}

fn portable_stem(name: &str) -> Option<String> {
    let stem: String = name
        .strip_suffix(".#dr")?
        .chars()
        .take(128)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if !stem.bytes().any(|ch| ch.is_ascii_alphanumeric()) {
        return None;
    }
    let device = stem.to_ascii_uppercase();
    let numbered_device = device
        .strip_prefix("COM")
        .or_else(|| device.strip_prefix("LPT"))
        .is_some_and(|n| matches!(n.as_bytes(), [b'0'..=b'9']));
    let offset_like = stem.len() >= 8 && stem.bytes().all(|ch| ch.is_ascii_hexdigit());
    if numbered_device || offset_like || matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return Some(format!("_{stem}"));
    }
    Some(stem)
}

fn unique_name(stem: &str, ext: &str, offset: usize, used: &mut HashSet<String>) -> String {
    let plain = format!("{stem}.{ext}");
    if used.insert(plain.to_ascii_lowercase()) {
        return plain;
    }
    let suffix = format!("{stem}-{offset:08x}");
    let mut filename = format!("{suffix}.{ext}");
    let mut attempt = 2;
    while !used.insert(filename.to_ascii_lowercase()) {
        filename = format!("{suffix}-{attempt}.{ext}");
        attempt += 1;
    }
    filename
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_valid_drawable_name_pointers() {
        let mut page = [0; 0x100];
        page[0xa8..0xb0].copy_from_slice(&0x5000_00c0_u64.to_le_bytes());
        page[0xc0..0xcc].copy_from_slice(b"lamp_01.#dr\0");
        assert_eq!(drawable_name(&page), Some("lamp_01.#dr"));
        assert_eq!(drawable_name(&page[..0xcb]), None);
        page[0xab] = 0x60;
        assert_eq!(drawable_name(&page), None);
    }
    #[test]
    fn recovered_names_use_offsets_only_for_collisions() {
        let mut used = HashSet::new();
        assert_eq!(unique_name("Lamp", "rsc", 0x100, &mut used), "Lamp.rsc");
        assert_eq!(
            unique_name("lamp", "rsc", 0x200, &mut used),
            "lamp-00000200.rsc"
        );
        assert_eq!(portable_stem("00000100.#dr").as_deref(), Some("_00000100"));
        assert_eq!(portable_stem("CON.#dr").as_deref(), Some("_CON"));
    }
}
