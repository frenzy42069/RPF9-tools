//! offs-compatible index output: `index.tsv` with columns
//! `archive, offset, length, size, parts, ext, kind, note`.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use crate::kinds::Kind;
use crate::scan::Scan;
use crate::stream::Stream;

fn note_for(s: &Stream) -> String {
    match s.kind {
        Kind::Movie => format!("Bink2 movie, stored length {}", s.stored_len),
        Kind::Adat => "ADAT audio bank".to_string(),
        Kind::Png => "PNG image".to_string(),
        Kind::PsIn => format!("PSIN catalog, decompressed {} bytes", s.decoded_len),
        Kind::Rsc9 => format!("meshlet resource, decompressed {} bytes", s.decoded_len),
        _ => String::new(),
    }
}

pub fn write_index_tsv(path: &Path, scan: &Scan) -> io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?);
    write_index(&mut out, scan)?;
    out.flush()
}

pub fn write_index(out: &mut impl Write, scan: &Scan) -> io::Result<()> {
    out.write_all(streams_tsv_header().as_bytes())?;
    for (i, a) in scan.archives.iter().enumerate() {
        for s in scan.owned_streams(i) {
            let (ext, kind) = s.kind.label();
            let note = note_for(&s);
            writeln!(
                out,
                "{:#x}\t{:#x}\t{}\t{}\t1\t{}\t{}\t{}",
                a.header.at, s.at, s.stored_len, s.decoded_len, ext, kind, note
            )?;
        }
    }
    Ok(())
}

pub fn streams_tsv_header() -> &'static str {
    "archive\toffset\tlength\tsize\tparts\text\tkind\tnote\n"
}
