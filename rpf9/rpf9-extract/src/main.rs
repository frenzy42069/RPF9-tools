//! `rpf9-extract` — static dearchivator (offs/rpf9-extract-compatible output).
//!
//! Decryption-less: recovers resources from zlib streams and carved media;
//! the encrypted RPF9 directory (original names) is not decoded.

use std::fs::{self, File};
use std::io::{self, Write};

mod range;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use memmap2::Mmap;
use rpf9_core::kinds::Kind;
use rpf9_core::scan::Scan;
use rpf9_core::stream;
use rpf9_core::{index, names, report};

const USAGE: &str = "\
usage: rpf9-extract [-n|--list] [-v|--verbose] [--offset-names] <rpf> [outdir]
       rpf9-extract --index-stdout <rpf>
       rpf9-extract --range-stdout <start> <length> <rpf>

Extracts to outdir (default: out/<file stem>). Requires an empty outdir.
Uses embedded filenames where available and stream offsets otherwise.
--offset-names uses offset-based filenames for all entries.
Writes report.txt and index.tsv (archive, offset, length, size, parts,
ext, kind, note).
";

fn main() -> ExitCode {
    let mut list = false;
    let mut verbose = false;
    let mut index_stdout = false;
    let mut recover_names = None;
    let mut range_args = None;
    let mut paths = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-n" | "--list" => {
                list = true;
                continue;
            }
            "-v" | "--verbose" => {
                verbose = true;
                continue;
            }
            "--range-stdout" => {
                let Some(start) = args.next().and_then(|v| range::parse_number(&v)) else {
                    return fail("--range-stdout requires a valid start and length");
                };
                let Some(length) = args.next().and_then(|v| range::parse_number(&v)) else {
                    return fail("--range-stdout requires a valid start and length");
                };
                range_args = Some((start, length));
                continue;
            }
            "--recover-names" => {
                recover_names = Some(true);
                continue;
            }
            "--offset-names" => {
                recover_names = Some(false);
                continue;
            }
            "--index-stdout" => {
                index_stdout = true;
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
    if range_args.is_some()
        && (index_stdout || list || verbose || recover_names.is_some() || !rest.is_empty())
    {
        return fail("--range-stdout takes a start, length and archive path");
    }
    if index_stdout && (list || verbose || recover_names.is_some() || !rest.is_empty()) {
        return fail("--index-stdout takes only an archive path");
    }
    if recover_names.is_some() && list {
        return fail("filename options require extraction");
    }
    let out: PathBuf = match rest.first() {
        Some(out) => PathBuf::from(out),
        None => Path::new("out").join(Path::new(file).file_stem().unwrap_or_default()),
    };
    if let Some((start, length)) = range_args {
        return match range::write(file, start, length) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e.to_string()),
        };
    }
    run(
        file,
        &out,
        list,
        verbose,
        index_stdout,
        recover_names.unwrap_or(true),
    )
}

fn run(
    file: &str,
    out: &Path,
    list: bool,
    verbose: bool,
    index_stdout: bool,
    recover_names: bool,
) -> ExitCode {
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

    if index_stdout {
        let mut stdout = io::BufWriter::new(io::stdout().lock());
        return match index::write_index(&mut stdout, &scan).and_then(|()| stdout.flush()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e.to_string()),
        };
    }

    println!("{}", report::render(file, data.len(), &scan));
    if verbose {
        print_headers(&scan);
    }
    if list {
        return ExitCode::SUCCESS;
    }
    if let Err(e) = extract(&data, &scan, out, file, recover_names) {
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
            a.header.at,
            a.header.len,
            a.header.split,
            a.header.trailer_len,
            a.header.w28,
            a.header.files,
            a.header.id
        );
    }
}

fn extract(
    data: &[u8],
    scan: &Scan,
    root: &Path,
    file: &str,
    recover_names: bool,
) -> io::Result<()> {
    if root
        .read_dir()
        .is_ok_and(|mut entries| entries.next().is_some())
    {
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
        let mut used_names = names::Filenames::default();
        for s in &streams {
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
            let name = recover_names
                .then(|| names::embedded_stem(s, &buf))
                .flatten();
            let filename = used_names.for_stream(s, name.as_deref());
            fs::write(dir.join(filename), buf)?;
        }
    }
    index::write_index_tsv(&root.join("index.tsv"), scan)?;
    fs::write(
        root.join("report.txt"),
        report::render(file, data.len(), scan),
    )?;
    Ok(())
}
