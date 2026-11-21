//! `rpf9-extract` — static dearchivator (offs/rpf9-extract-compatible output).
//!
//! Decryption-less: recovers resources from zlib streams and carved media;
//! the encrypted RPF9 directory (original names) is not decoded.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use memmap2::Mmap;
use rpf9_core::kinds::Kind;
use rpf9_core::scan::Scan;
use rpf9_core::stream;
use rpf9_core::{index, report};

const USAGE: &str = "\
usage: rpf9-extract [-n|--list] [-v|--verbose] <rpf> [outdir]

Extracts to outdir (default: out/<file stem>). Requires an empty outdir.
Files are named by stream offset: `h<archive:06x>/<at:08x>.<ext>`.
Writes report.txt and index.tsv (archive, offset, length, size, parts,
ext, kind, note).
";

fn main() -> ExitCode {
    let mut list = false;
    let mut verbose = false;
    let mut paths = Vec::new();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "-n" | "--list" => {
                list = true;
                continue;
            }
            "-v" | "--verbose" => {
                verbose = true;
                continue;
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            s if s.starts_with('-') => {
                eprintln!("rpf9-extract: unknown option {s}\n{USAGE}");
                return ExitCode::FAILURE;
            }
            _ => {}
        }
        paths.push(a);
    }
    let [file, rest @ ..] = paths.as_slice() else {
        eprintln!("rpf9-extract: missing <rpf>\n{USAGE}");
        return ExitCode::FAILURE;
    };
    if rest.len() > 1 {
        eprintln!("rpf9-extract: too many arguments\n{USAGE}");
        return ExitCode::FAILURE;
    }
    let out: PathBuf = match rest.first() {
        Some(out) => PathBuf::from(out),
        None => Path::new("out").join(Path::new(file).file_stem().unwrap_or_default()),
    };
    run(file, &out, list, verbose)
}

fn run(file: &str, out: &Path, list: bool, verbose: bool) -> ExitCode {
    let f = match File::open(file) {
        Ok(f) => f,
        Err(e) => return fail(&format!("open {file}: {e}")),
    };
    // SAFETY: the map is read-only; the file is not modified while inspected.
    let data = match unsafe { Mmap::map(&f) } {
        Ok(d) => d,
        Err(e) => return fail(&format!("map {file}: {e}")),
    };
    let scan = Scan::new(&data);

    println!("{}", report::render(file, data.len(), &scan));
    if verbose {
        print_headers(&scan);
    }
    if list {
        return ExitCode::SUCCESS;
    }
    if let Err(e) = extract(&data, &scan, out, file) {
        return fail(&e.to_string());
    }
    println!("wrote to {}", out.display());
    ExitCode::SUCCESS
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("rpf9-extract: {msg}");
    ExitCode::FAILURE
}

fn print_headers(scan: &Scan) {
    for a in &scan.archives {
        println!(
            "archive {:#x}: A={:#x} B={:#x} C={:#x} D={:#x} N={} id={:#018x}",
            a.header.at, a.header.len, a.header.split, a.header.trailer_len, a.header.w28,
            a.header.files, a.header.id
        );
    }
}

fn extract(data: &[u8], scan: &Scan, root: &Path, file: &str) -> io::Result<()> {
    if root.read_dir().is_ok_and(|mut entries| entries.next().is_some()) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} is not empty", root.display()),
        ));
    }
    fs::create_dir_all(root)?;
    for (i, a) in scan.archives.iter().enumerate() {
        let streams = scan.owned_streams(i);
        if streams.is_empty() {
            continue;
        }
        let dir = root.join(a.name());
        fs::create_dir_all(&dir)?;
        for s in &streams {
            let ext = s.kind.label().0;
            let path = dir.join(format!("{:08x}.{ext}", s.at));
            let buf = match s.kind {
                Kind::Movie | Kind::Adat | Kind::Png | Kind::Riff { .. } => {
                    data[s.at..s.at + s.stored_len].to_vec()
                }
                _ => {
                    let mut out = Vec::new();
                    if stream::inflate(data, s.at, &mut out).is_none() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("inflate failed at {:#x}", s.at),
                        ));
                    }
                    out
                }
            };
            fs::write(path, buf)?;
        }
    }
    index::write_index_tsv(&root.join("index.tsv"), scan)?;
    fs::write(root.join("report.txt"), report::render(file, data.len(), scan))?;
    Ok(())
}
