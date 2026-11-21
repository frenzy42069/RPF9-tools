//! Media carving by signature (ported from offs/rpf9-extract `carve.rs`):
//! movies (KB2/BINK), ADAT banks, PNG images, RIFF files.

use std::ops::Range;

use crate::bytes::{be32, le16, le32};
use crate::kinds::Kind;
use crate::stream::Stream;

/// Carve media in `region` not covered by known ranges (`covered`, sorted by
/// start).
pub fn carve_media(data: &[u8], region: Range<usize>, covered: &[Range<usize>]) -> Vec<Stream> {
    let mut out = Vec::new();
    let mut at = region.start;
    while at + 16 <= region.end {
        let ci = covered.partition_point(|r| r.start <= at);
        if ci > 0 && covered[ci - 1].end > at {
            at += 16;
            continue;
        }
        if let Some(len) = bink_len(data, at) {
            out.push(Stream { at, stored_len: len, decoded_len: 0, kind: Kind::Movie });
        } else if let Some((len, kind)) = awc_len(data, at) {
            out.push(Stream { at, stored_len: len, decoded_len: 0, kind });
        } else if let Some(len) = png_len(data, at) {
            out.push(Stream { at, stored_len: len, decoded_len: 0, kind: Kind::Png });
        } else if let Some((len, kind)) = riff_len(data, at) {
            out.push(Stream { at, stored_len: len, decoded_len: 0, kind });
        }
        at += 16;
    }
    out
}

/// Bink2/Bink: `data[..3]` is `BIK` or `KB2`, lowercase revision byte,
/// `len = le32(at+4) + 8` within range, plausible frame count.
fn bink_len(data: &[u8], at: usize) -> Option<usize> {
    match data.get(at..at + 3)? {
        b"BIK" | b"KB2" => {}
        _ => return None,
    }
    let len = le32(data, at + 4)? as usize + 8;
    let frames = le32(data, at + 8)?;
    let revision = data.get(at + 3).is_some_and(u8::is_ascii_lowercase);
    let plausible = (1..=1 << 20).contains(&frames) && (12..=data.len()).contains(&len);
    (revision && plausible).then_some(len)
}

/// ADAT audio bank (ported from offs `awc()`).
fn awc_len(data: &[u8], at: usize) -> Option<(usize, Kind)> {
    const KNOWN_FLAGS: [u32; 4] = [0xff02_0001, 0xff21_0001, 0xff25_0001, 0xff61_0001];
    const HAS_WORD: u32 = 0x40_0000;
    const HAS_FIRST_TAGS: u32 = 0x01_0000;
    const HAS_TABLE: u32 = 0x20_0000;
    const DATA_TAG: u64 = 0x55;

    if data.get(at..at + 4) != Some(b"ADAT") {
        return None;
    }
    let (flags, streams) = (le32(data, at + 4)?, le32(data, at + 8)? as usize);
    if !KNOWN_FLAGS.contains(&flags) || streams > 0x10_000 {
        return None;
    }
    let header_len = le32(data, at + 12)? as usize;
    let header = data.get(at..at + header_len)?;

    let first_tags = 16 + if flags & HAS_WORD != 0 { 4 } else { 0 };
    let table = first_tags + if flags & HAS_FIRST_TAGS != 0 { 2 * streams } else { 0 };
    let ids = table + if flags & HAS_TABLE != 0 { 4 * (streams + 1) } else { 0 };
    let tags = ids + 4 * streams;

    let mut count = 0usize;
    for stream in 0..streams {
        let first_tag = le16(header, first_tags + 2 * stream).map(usize::from);
        if flags & HAS_FIRST_TAGS != 0 && first_tag? != count {
            return None;
        }
        count += (le32(header, ids + 4 * stream)? >> 29) as usize;
    }
    if tags + 8 * count != header.len() {
        return None;
    }

    let (mut len, mut has_data) = (header_len, false);
    for chunk in header[tags..].as_chunks::<8>().0 {
        let tag = u64::from_le_bytes(*chunk);
        let (kind, size, at2) = (tag >> 56, tag >> 28 & 0xfff_ffff, tag & 0xfff_ffff);
        let (size, at2) = (size as usize, at2 as usize);
        if (size > 0 && at2 < header.len()) || at2 + size > data.len() {
            return None;
        }
        len = len.max(at2 + size);
        has_data |= kind == DATA_TAG && size > 0;
    }
    if !has_data && streams == 0 {
        return None;
    }
    Some((len, Kind::Adat))
}

/// PNG image: signature + IHDR + walk to IEND.
fn png_len(data: &[u8], at: usize) -> Option<usize> {
    if data.get(at..at + 8) != Some(b"\x89PNG\r\n\x1a\n")
        || data.get(at + 8..at + 16) != Some(b"\0\0\0\rIHDR")
    {
        return None;
    }
    let mut len = 8;
    loop {
        let size = be32(data, at + len)? as usize;
        let chunk = data.get(at + len + 4..at + len + 8)?;
        len += size + 12;
        if chunk == b"IEND" {
            break;
        }
        if len > data.len() - at {
            return None;
        }
    }
    Some(len)
}

/// RIFF: WAVE/AVI/WEBP with a plausible length.
fn riff_len(data: &[u8], at: usize) -> Option<(usize, Kind)> {
    if data.get(at..at + 4) != Some(b"RIFF") {
        return None;
    }
    let len = le32(data, at + 4)? as usize + 8;
    if at + len > data.len() {
        return None;
    }
    let kind = match (data.get(at + 8..at + 12)?, data.get(at + 12..at + 16)?) {
        (b"WAVE", b"fmt ") => Kind::Riff { ext: "wav", name: "WAVE audio" },
        (b"AVI ", b"LIST") => Kind::Riff { ext: "avi", name: "AVI video" },
        (b"WEBP", _) => Kind::Riff { ext: "webp", name: "WebP image" },
        _ => return None,
    };
    Some((len, kind))
}
