//! zlib stream recovery: decompression with Adler-32 verification (flate2),
//! plus the aligned-offset walk used by the scanner.

use flate2::{Decompress, FlushDecompress, Status};

use crate::kinds::{classify, Kind};
use crate::util::align16;

#[derive(Debug, Clone, Copy)]
pub struct Stream {
    /// Absolute stored start (inside an archive body).
    pub at: usize,
    /// Compressed length in bytes.
    pub stored_len: usize,
    /// Decompressed length (0 for carved media).
    pub decoded_len: usize,
    pub kind: Kind,
}

/// zlib RFC 1950 magic check: `CMF & 0x0F == 8`, `CMF >> 4 <= 7`,
/// `(256*CMF + FLG) % 31 == 0`, no FDICT.
pub fn is_zlib_magic(data: &[u8], at: usize) -> bool {
    let (Some(&cmf), Some(&flg)) = (data.get(at), data.get(at + 1)) else {
        return false;
    };
    (cmf & 0x0F) == 8 && (cmf >> 4) <= 7 && u16::from_be_bytes([cmf, flg]) % 31 == 0
}

/// Decompress the zlib stream at `at` into `out`. Returns the stored length
/// on success (flate2 verifies the Adler-32 on stream end).
pub fn inflate(data: &[u8], at: usize, out: &mut Vec<u8>) -> Option<usize> {
    if !is_zlib_magic(data, at) {
        return None;
    }
    let mut z = Decompress::new(true);
    z.reset(true);
    let stream = &data[at..];
    loop {
        let read = z.total_in() as usize;
        out.reserve(1 << 16);
        match z.decompress_vec(&stream[read..], out, FlushDecompress::None) {
            Ok(Status::StreamEnd) => return Some(z.total_in() as usize),
            Ok(Status::Ok) => {
                // No progress and no input left: not a complete stream.
                if z.total_in() as usize == read {
                    return None;
                }
            }
            _ => return None,
        }
    }
}

/// Decompress the stream at `at` and classify by decompressed signature.
pub fn classify_stream(data: &[u8], at: usize) -> Option<Stream> {
    let mut out = Vec::new();
    let stored = inflate(data, at, &mut out)?;
    Some(Stream {
        at,
        stored_len: stored,
        decoded_len: out.len(),
        kind: classify(&out),
    })
}

/// Walk `[start, end)` at 16-byte steps, recording every complete zlib stream.
pub fn walk(data: &[u8], start: usize, end: usize) -> Vec<Stream> {
    walk_with(data, start, end, |_, _| {})
}

/// Run a callback for each verified stream while its decoded bytes are available.
pub fn walk_with(
    data: &[u8],
    start: usize,
    end: usize,
    mut on_stream: impl FnMut(&Stream, &[u8]),
) -> Vec<Stream> {
    let mut streams = Vec::new();
    let mut out: Vec<u8> = Vec::new();
    let mut at = start;
    while at + 16 <= end {
        if !is_zlib_magic(data, at) {
            at += 16;
            continue;
        }
        out.clear();
        match inflate(data, at, &mut out) {
            Some(stored) => {
                let stream = Stream {
                    at,
                    stored_len: stored,
                    decoded_len: out.len(),
                    kind: classify(&out),
                };
                on_stream(&stream, &out);
                streams.push(stream);
                at = align16(at + stored);
            }
            None => at += 16,
        }
    }
    streams
}
