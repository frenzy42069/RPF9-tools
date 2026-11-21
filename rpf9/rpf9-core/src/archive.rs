//! RPF9 archives: header discovery + extents (offs/rpf9-extract model).

use std::ops::Range;

use memchr::memmem;

use crate::header::Header;

#[derive(Debug, Clone)]
pub struct Archive {
    pub header: Header,
    /// `[header.at + A, trailer.start)` — the payload region.
    pub body: Range<usize>,
    /// Candidate terminal region `[end - C, end)`.
    pub trailer: Range<usize>,
}

impl Archive {
    /// `[at, trailer.end)` — the whole archive extent.
    pub fn extent(&self) -> Range<usize> {
        self.header.at..self.trailer.end
    }

    /// Structure ranges: `[at, body.start)` and the trailer.
    pub fn structure(&self) -> [Range<usize>; 2] {
        [self.header.at..self.body.start, self.trailer.clone()]
    }

    /// offs-style name: `h<at:06x>`.
    pub fn name(&self) -> String {
        format!("h{:06x}", self.header.at)
    }
}

/// Find all valid 9FPR headers, sorted by offset.
pub fn find_headers(data: &[u8]) -> Vec<Header> {
    let mut out = Vec::new();
    for at in memmem::find_iter(data, b"9FPR") {
        if let Some(h) = Header::parse(data, at) {
            out.push(h);
        }
    }
    out.sort_by_key(|h| h.at);
    out
}

/// Place one archive: end = last nonzero byte before `limit`; trailer =
/// `[end - C, end)`; body = `[at + A, trailer.start)`.
pub fn place(data: &[u8], header: Header, limit: usize) -> Archive {
    let start = header.at + header.len;
    let limit = limit.max(start);
    let mut end = start;
    if start < limit {
        let last = data[start..limit].iter().rposition(|&b| b != 0);
        end = start + last.map_or(0, |i| i + 1);
    }
    let end = end.next_multiple_of(16).min(limit);
    let trailer = end.saturating_sub(header.trailer_len).max(start)..end;
    Archive { header, body: start..trailer.start, trailer }
}

/// All archives: outer + nested (offs `layout`).
pub fn find_archives(data: &[u8]) -> Vec<Archive> {
    let headers = find_headers(data);
    headers
        .iter()
        .enumerate()
        .map(|(i, h)| place(data, *h, headers.get(i + 1).map_or(data.len(), |n| n.at)))
        .collect()
}
