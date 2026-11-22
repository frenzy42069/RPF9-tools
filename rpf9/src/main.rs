mod assets;
mod video;

use std::{
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use anyhow::{bail, ensure, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use rpf9_convert::{AudioOptions, ModelFormat, TextureFormat};

use assets::{Asset, Kind};
use video::VideoFormat;

#[derive(Parser)]
#[command(version, about = "Catalog and export assets from RPF9 archives")]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Run the raw archive extractor with its original arguments.
    Extract {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Mount an archive through the installed FUSE tool.
    Mount {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Find assets in an archive or inspect a standalone file.
    List {
        #[command(flatten)]
        selection: Selection,
        /// Write a machine-readable JSON array.
        #[arg(long)]
        json: bool,
    },
    /// Export matching assets to an output directory.
    Export {
        #[command(flatten)]
        selection: Selection,
        #[arg(short, long)]
        output_dir: PathBuf,
        /// Model: glb/gltf/obj, texture: png/jpeg/webp/bmp/tga, audio: original/wav,
        /// video: bk2/mp4/mkv/webm, localization: jsonl/csv.
        #[arg(short, long)]
        format: Option<String>,
        /// Use FFmpeg from PATH, or pass --ffmpeg=PATH to an executable.
        /// Xbox Bink2 requires a Bink2-capable build for video conversion.
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "ffmpeg")]
        ffmpeg: Option<PathBuf>,
        /// ADAT extractor executable, otherwise look for rpf9-extract-audio on PATH.
        #[arg(long)]
        audio_extractor: Option<PathBuf>,
    },
}

#[derive(Args)]
struct Selection {
    input: PathBuf,
    #[arg(short, long, value_enum)]
    kind: Kind,
    /// Beginning of the archive search window, in decimal or hexadecimal.
    #[arg(long, default_value = "0", value_parser = parse_offset)]
    start: u64,
    /// Maximum bytes to scan. Use --all to scan to the end.
    #[arg(long, default_value = "0x10000000", value_parser = parse_offset)]
    scan_bytes: u64,
    /// Scan to EOF and remove the default result limit.
    #[arg(long)]
    all: bool,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    limit: Option<u32>,
    /// Case-insensitive substring of the asset name.
    #[arg(long)]
    name: Option<String>,
    /// Select a known archive entry directly.
    #[arg(long, value_parser = parse_offset)]
    offset: Option<u64>,
    /// Select a zero-based index in a texture pack.
    #[arg(long)]
    entry: Option<usize>,
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

fn run(action: Action) -> Result<()> {
    match action {
        Action::Extract { args } => run_tool("rpf9-extract", args),
        Action::Mount { args } => run_tool("rpf9-fuse", args),
        Action::List { selection, json } => {
            let entries = assets::find(&selection)?;
            let mut out = io::BufWriter::new(io::stdout().lock());
            if json {
                serde_json::to_writer(&mut out, &entries)?;
                writeln!(out)?;
            } else {
                writeln!(out, "KIND\tOFFSET\tENTRY\tNAME")?;
                for asset in entries {
                    let offset = asset
                        .offset
                        .map_or_else(|| "-".into(), |off| format!("{off:#x}"));
                    let entry = asset
                        .entry
                        .map_or_else(|| "-".into(), |idx| idx.to_string());
                    writeln!(
                        out,
                        "{}\t{}\t{}\t{}",
                        asset.kind.label(),
                        offset,
                        entry,
                        asset.name
                    )?;
                }
            }
            out.flush()?;
            Ok(())
        }
        Action::Export {
            selection,
            output_dir,
            format,
            ffmpeg,
            audio_extractor,
        } => {
            let format = ExportFormat::parse(selection.kind, format.as_deref())?;
            let entries = assets::find(&selection)?;
            ensure!(
                !entries.is_empty(),
                "no matching assets in the selected range"
            );
            fs::create_dir_all(&output_dir)
                .with_context(|| format!("create {}", output_dir.display()))?;
            let mut failed = 0;
            for asset in &entries {
                let target = output_dir.join(asset.output_name(format.extension()));
                match format.export(
                    asset,
                    &target,
                    ffmpeg.as_deref(),
                    audio_extractor.as_deref(),
                ) {
                    Ok(()) => println!("{}", target.display()),
                    Err(error) => {
                        eprintln!("{}: {error:#}", asset.name);
                        failed += 1;
                    }
                }
            }
            ensure!(failed == 0, "{failed} of {} exports failed", entries.len());
            Ok(())
        }
    }
}

fn run_tool(name: &str, args: Vec<OsString>) -> Result<()> {
    let filename = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let sibling = std::env::current_exe()?.with_file_name(&filename);
    let command = if sibling.is_file() {
        sibling
    } else {
        PathBuf::from(filename)
    };
    let status = Command::new(&command)
        .args(args)
        .status()
        .with_context(|| format!("launch {name}; build it beside rpf9 or put it on PATH"))?;
    ensure!(status.success(), "{name} exited with {status}");
    Ok(())
}

#[derive(Clone, Copy)]
enum ExportFormat {
    Model(ModelFormat),
    Texture(TextureFormat),
    Audio(bool),
    Video(VideoFormat),
    Localization(rpf9_convert::LocalizationFormat),
}

impl ExportFormat {
    fn parse(kind: Kind, format: Option<&str>) -> Result<Self> {
        fn value<T: ValueEnum>(format: Option<&str>, default: &str) -> Result<T> {
            T::from_str(format.unwrap_or(default), true).map_err(anyhow::Error::msg)
        }
        Ok(match kind {
            Kind::Model => Self::Model(value(format, "glb")?),
            Kind::Texture => Self::Texture(value(format, "png")?),
            Kind::Video => Self::Video(value(format, "mp4")?),
            Kind::Localization => Self::Localization(value(format, "jsonl")?),
            Kind::Audio => match format.unwrap_or("original") {
                "original" => Self::Audio(false),
                "wav" => Self::Audio(true),
                other => bail!("unsupported audio format {other}; choose original or wav"),
            },
        })
    }

    fn extension(self) -> Option<&'static str> {
        match self {
            Self::Model(fmt) => Some(fmt.extension()),
            Self::Texture(fmt) => Some(fmt.extension()),
            Self::Video(fmt) => Some(fmt.extension()),
            Self::Localization(fmt) => Some(match fmt {
                rpf9_convert::LocalizationFormat::Jsonl => "jsonl",
                rpf9_convert::LocalizationFormat::Csv => "csv",
            }),
            Self::Audio(_) => None,
        }
    }

    fn export(
        self,
        asset: &Asset,
        target: &Path,
        ffmpeg: Option<&Path>,
        audio_extractor: Option<&Path>,
    ) -> Result<()> {
        match self {
            Self::Model(format) => {
                rpf9_convert::export_model(&asset.source, asset.offset, target, format)
            }
            Self::Texture(format) => match asset.offset {
                Some(offset) => rpf9_convert::export_dictionary_texture(
                    &asset.source,
                    offset,
                    asset.entry.context("missing texture index")?,
                    target,
                    format,
                    ffmpeg,
                ),
                None => {
                    rpf9_convert::export_texture(&asset.source, asset.entry, target, format, ffmpeg)
                }
            },
            Self::Audio(wav) => {
                ensure!(
                    !target.exists(),
                    "output directory already exists: {}",
                    target.display()
                );
                rpf9_convert::export_audio(&AudioOptions {
                    input: &asset.source,
                    output: target,
                    bank: asset.offset,
                    extractor: audio_extractor,
                    wav_ffmpeg: wav.then(|| ffmpeg.unwrap_or(Path::new("ffmpeg"))),
                })
            }
            Self::Video(format) => {
                video::export(asset, target, format, ffmpeg.unwrap_or(Path::new("ffmpeg")))
            }
            Self::Localization(format) => {
                rpf9_convert::export_localization(&asset.source, asset.offset, target, format)
            }
        }
    }
}

fn parse_offset(value: &str) -> Result<u64, std::num::ParseIntError> {
    match value.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => value.parse(),
    }
}
