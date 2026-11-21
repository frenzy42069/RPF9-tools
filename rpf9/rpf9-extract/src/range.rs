use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use memmap2::Mmap;
use rpf9_core::{index, media, stream};

const LOOKAHEAD: usize = 16 * 1024 * 1024;

pub fn parse_number(value: &str) -> Option<usize> {
    if let Some(hex) = value.strip_prefix("0x") {
        usize::from_str_radix(hex, 16).ok()
    } else {
        value.parse().ok()
    }
}

pub fn write(path: impl AsRef<Path>, start: usize, length: usize) -> io::Result<()> {
    if length == 0 || !start.is_multiple_of(16) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "range must be nonempty and start aligned to 16 bytes",
        ));
    }
    let file = File::open(path)?;
    // SAFETY: the file is mapped read-only and is not modified during the scan.
    let data = unsafe { Mmap::map(&file)? };
    if start >= data.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "range start is outside the archive",
        ));
    }
    let end = start.saturating_add(length).min(data.len());
    let limited = &data[..end.saturating_add(LOOKAHEAD).min(data.len())];
    let mut streams = stream::walk(limited, start, end);
    let covered: Vec<_> = streams.iter().map(|s| s.at..s.at + s.stored_len).collect();
    streams.extend(media::carve_media(&data, start..end, &covered));
    streams.sort_by_key(|s| s.at);
    let mut out = BufWriter::new(io::stdout().lock());
    out.write_all(index::streams_tsv_header().as_bytes())?;
    for item in streams {
        let (ext, kind) = item.kind.label();
        writeln!(
            out,
            "{start:#x}\t{:#x}\t{}\t{}\t1\t{}\t{}\t",
            item.at, item.stored_len, item.decoded_len, ext, kind
        )?;
    }
    out.flush()
}
