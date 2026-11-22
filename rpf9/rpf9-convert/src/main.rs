use std::{path::PathBuf, process::ExitCode};

use anyhow::Result;
use clap::{Parser, Subcommand};
use rpf9_convert::{export_model, export_texture, list_models, ModelFormat, TextureFormat};

#[derive(Parser)]
#[command(version, about = "Convert model and texture files")]
struct Cli {
    #[command(subcommand)]
    command: Convert,
}

#[derive(Subcommand)]
enum Convert {
    /// List convertible drawables in a bounded archive range as JSON.
    ListModels {
        input: PathBuf,
        /// Start scanning at this byte offset (decimal or 0x hexadecimal).
        #[arg(long, value_parser = parse_offset, default_value = "0")]
        start: u64,
        /// Maximum bytes to scan, in decimal or 0x hexadecimal.
        #[arg(long, value_parser = parse_offset, default_value = "0x10000000")]
        scan_bytes: u64,
        /// Stop after this many drawable models.
        #[arg(long, default_value_t = 100, value_parser = parse_limit)]
        limit: usize,
    },
    /// Convert a model to GLB, glTF 2.0, or OBJ.
    Model {
        input: PathBuf,
        #[arg(short, long, value_enum, default_value = "glb")]
        format: ModelFormat,
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Byte offset of a drawable stream in an RPF, in decimal or 0x hex.
        #[arg(long, value_parser = parse_offset)]
        offset: Option<u64>,
    },
    /// Convert a texture to PNG, JPEG, WebP, BMP, or TGA.
    Texture {
        input: PathBuf,
        #[arg(short, long, value_enum, default_value = "png")]
        format: TextureFormat,
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Zero-based entry in a texture container.
        #[arg(long)]
        entry: Option<usize>,
        /// Encode with FFmpeg from PATH. Pass --ffmpeg=PATH to use a specific executable.
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "ffmpeg")]
        ffmpeg: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse().command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Convert) -> Result<()> {
    match command {
        Convert::ListModels {
            input,
            start,
            scan_bytes,
            limit,
        } => {
            let models = list_models(&input, start, scan_bytes, limit)?;
            serde_json::to_writer(std::io::stdout().lock(), &models)?;
            println!();
            Ok(())
        }
        Convert::Model {
            input,
            format,
            output,
            offset,
        } => {
            let output = output.unwrap_or_else(|| match offset {
                Some(offset) => input.with_extension(format!("{offset:x}.{}", format.extension())),
                None => input.with_extension(format.extension()),
            });
            export_model(&input, offset, &output, format)
        }
        Convert::Texture {
            input,
            format,
            output,
            entry,
            ffmpeg,
        } => {
            let extension = format.extension();
            let output = output.unwrap_or_else(|| match entry {
                Some(index) => input.with_extension(format!("{index}.{extension}")),
                None => input.with_extension(extension),
            });
            export_texture(&input, entry, &output, format, ffmpeg.as_deref())
        }
    }
}

fn parse_limit(value: &str) -> Result<usize, String> {
    let parsed = value.parse::<usize>().map_err(|error| error.to_string())?;
    if parsed == 0 {
        return Err("limit must be positive".to_owned());
    }
    Ok(parsed)
}

fn parse_offset(value: &str) -> Result<u64, std::num::ParseIntError> {
    match value.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => value.parse(),
    }
}
