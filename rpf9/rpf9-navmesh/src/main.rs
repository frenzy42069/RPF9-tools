//! `rpf9-navmesh` — navmesh resources → Wavefront OBJ.
//!
//! Usage: `rpf9-navmesh convert <input> [-o output.obj] [--offset N] [-v]`
//!
//! `<input>` is an extracted navmesh resource (e.g. `results/navmeshes/h*/…/*.rsc`)
//! or any file containing one; `--offset` reads raw bytes at a byte offset of
//! the input file (decimal or `0x` hex). The discovered mesh is written as
//! `v`/`f` lines; vertices come first, so OBJ indices are 1-based.

mod mesh;

use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use memmap2::Mmap;

#[derive(Parser)]
#[command(version, about = "Convert navmesh resources to Wavefront OBJ")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Discover the navmesh mesh in an extracted resource and write it as OBJ.
    Convert {
        /// Navmesh resource file (extracted `.rsc`, or any file with one inside).
        input: PathBuf,
        /// Output OBJ path (default: `<input>.obj`).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Byte offset of the resource in the input file (decimal or `0x` hex).
        #[arg(long, value_parser = parse_offset)]
        offset: Option<u64>,
        /// Print the parsed mesh summary to stderr.
        #[arg(short, long)]
        verbose: bool,
    },
}

fn main() -> ExitCode {
    let Cli { command } = Cli::parse();
    match command {
        Command::Convert { input, output, offset, verbose } => {
            match run(&input, output, offset, verbose) {
                Ok(out) => {
                    println!("wrote {}", out.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("rpf9-navmesh: {e:#}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

fn run(input: &Path, output: Option<PathBuf>, offset: Option<u64>, verbose: bool) -> Result<PathBuf> {
    let mm = mmap_input(input, offset)?;
    let data: &[u8] = if let Some(off) = offset {
        &mm[off as usize..]
    } else {
        &mm
    };
    let m = mesh::parse(data).context("no navmesh geometry found in the input")?;
    if verbose {
        eprintln!(
            "mesh: {} vertices, {} faces",
            m.vertices.len(),
            m.faces.len()
        );
    }
    let out_path = output.unwrap_or_else(|| input.with_extension("obj"));
    let mut buf = BufWriter::new(
        fs::File::create(&out_path).with_context(|| format!("create {}", out_path.display()))?,
    );
    mesh::write_obj(&mut buf, &m)?;
    buf.flush()?;
    Ok(out_path)
}

fn mmap_input(input: &Path, offset: Option<u64>) -> Result<Mmap> {
    let f = fs::File::open(input).with_context(|| format!("open {}", input.display()))?;
    let mm = unsafe { Mmap::map(&f) }.with_context(|| format!("map {}", input.display()))?;
    if let Some(off) = offset {
        let end = off as usize + 16; // sanity: at least a bit of data at the offset
        anyhow::ensure!(end <= mm.len(), "offset {off:#x} is past end of {}", input.display());
    }
    Ok(mm)
}

fn parse_offset(value: &str) -> Result<u64, std::num::ParseIntError> {
    match value.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => value.parse(),
    }
}
