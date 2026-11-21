//! RPF9 archive header parsing (`9FPR` signature, magic `0x52504639`).

use crate::bytes::{le16, le32, le64};

#[derive(Debug, Clone, Copy)]
pub struct Header {
    pub at: usize,
    /// `A`: end of the initial region (multiple of 16).
    pub len: usize,
    /// `B`: boundary of the second (table) region; `B < A`.
    pub split: usize,
    /// `C`: candidate trailer length.
    pub trailer_len: usize,
    /// Per-archive id (`u64le` at 0x10).
    pub id: u64,
    /// `N`: candidate entry count.
    pub files: u32,
    /// u16le at 0x20 and 0x22.
    pub w20: [u16; 2],
    /// u32le at 0x28.
    pub w28: u32,
    /// u16le at 0x2c and 0x2e.
    pub w2c: [u16; 2],
}

impl Header {
    /// Validate + parse the header at `at` in `data`. Mirrors offs/rpf9-extract:
    /// magic + 16-aligned `A`/`B`/`C`-derived tables that fit the file.
    pub fn parse(data: &[u8], at: usize) -> Option<Header> {
        let h = data.get(at..)?;
        if !h.starts_with(b"9FPR") {
            return None;
        }
        let len = le32(h, 0x04)? as usize;
        let split = le32(h, 0x08)? as usize;
        let trailer_len = le32(h, 0x0C)? as usize;
        // offs: tables = [split - HEADER, len - split]; both must be 16-aligned
        // and within the file.
        let tables = [split.checked_sub(0x40)?, len.checked_sub(split)?];
        if at + len > data.len() || tables.iter().any(|t| t % 16 != 0) {
            return None;
        }
        Some(Header {
            at,
            len,
            split,
            trailer_len,
            id: le64(h, 0x10)?,
            files: le32(h, 0x24)?,
            w20: [le16(h, 0x20)?, le16(h, 0x22)?],
            w28: le32(h, 0x28)?,
            w2c: [le16(h, 0x2c)?, le16(h, 0x2e)?],
        })
    }

    /// Size of the two front-region tables: `[0x40, B)` and `[B, A)`.
    pub fn table_records(&self) -> [usize; 2] {
        [self.split - 0x40, self.len - self.split]
    }
}
