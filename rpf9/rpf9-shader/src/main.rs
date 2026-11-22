//! `rpf9-shader` — decompile shader data: PSO caches and SGD5 blobs from
//! `shaders.rpf`.
//!
//! Usage: `rpf9-shader <input> [-o report.txt] [--offset N]`
//!
//! The input is a shader data file (an extracted PSO cache or SGD5 blob) or a
//! file read at a byte offset with `--offset` (decimal or `0x` hex).

mod pso;
mod sgd5;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use memmap2::Mmap;

#[derive(Parser)]
#[command(version, about = "Decompile shader data: PSO caches and SGD5 blobs")]
struct Cli {
    /// Input: PSO cache or SGD5 blob file.
    input: PathBuf,
    /// Byte offset in the input file (decimal or `0x` hex).
    #[arg(long, value_parser = parse_offset)]
    offset: Option<u64>,
    /// Write the decompilation report to this file instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rpf9-shader: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let mm = mmap(&cli.input)?;
    let data: &[u8] = if let Some(off) = cli.offset {
        &mm[off as usize..]
    } else {
        &mm
    };
    let report = if data.starts_with(b"SGD5") {
        sgd5::decompile(&cli.input, data)
    } else if data.starts_with(b"PS\x00\x09") {
        pso::decompile(&cli.input, data)
    } else {
        anyhow::bail!("unrecognized shader data (expected SGD5 or PS\\x00\\x09 magic)")
    };
    match &cli.output {
        Some(p) => fs::write(p, report).with_context(|| format!("write {}", p.display()))?,
        None => {
            std::io::stdout()
                .write_all(report.as_bytes())
                .context("write stdout")?;
        }
    }
    Ok(())
}

fn mmap(input: &Path) -> Result<Mmap> {
    let f = fs::File::open(input).with_context(|| format!("open {}", input.display()))?;
    unsafe { Mmap::map(&f) }.with_context(|| format!("map {}", input.display()))
}

fn parse_offset(value: &str) -> Result<u64, std::num::ParseIntError> {
    match value.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => value.parse(),
    }
}
