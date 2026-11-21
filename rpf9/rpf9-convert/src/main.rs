mod catalog;
mod drawable;
mod mesh;
mod model_output;
mod texture;

use std::{
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use image::{DynamicImage, ImageFormat, RgbaImage};
use tempfile::NamedTempFile;

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
        format: model_output::Format,
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

#[derive(Clone, Copy, ValueEnum)]
enum TextureFormat {
    Png,
    #[value(alias = "jpg")]
    Jpeg,
    Webp,
    Bmp,
    Tga,
}

impl TextureFormat {
    fn settings(self) -> (&'static str, ImageFormat, &'static str) {
        match self {
            Self::Png => ("png", ImageFormat::Png, "png"),
            Self::Jpeg => ("jpg", ImageFormat::Jpeg, "mjpeg"),
            Self::Webp => ("webp", ImageFormat::WebP, "libwebp"),
            Self::Bmp => ("bmp", ImageFormat::Bmp, "bmp"),
            Self::Tga => ("tga", ImageFormat::Tga, "targa"),
        }
    }
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
            let models = catalog::list(&input, start, scan_bytes, limit)?;
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
            let mesh = match offset {
                Some(offset) => drawable::load(&input, offset),
                None => mesh::load(&input),
            }
            .with_context(|| format!("read model {}", input.display()))?;
            write_output(&output, |file| {
                let mut writer = BufWriter::new(file.as_file_mut());
                model_output::write(&mut writer, &mesh, format)?;
                writer.flush()?;
                Ok(())
            })
        }
        Convert::Texture {
            input,
            format,
            output,
            entry,
            ffmpeg,
        } => {
            let (extension, image_format, codec) = format.settings();
            let output = output.unwrap_or_else(|| match entry {
                Some(index) => input.with_extension(format!("{index}.{extension}")),
                None => input.with_extension(extension),
            });
            let rgba = texture::load(&input, entry)
                .with_context(|| format!("read texture {}", input.display()))?;
            write_output(&output, |file| match ffmpeg {
                Some(executable) => encode_ffmpeg(&executable, rgba, codec, file.path()),
                None => encode_image(rgba, image_format, file),
            })
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

fn write_output(path: &Path, write: impl FnOnce(&mut NamedTempFile) -> Result<()>) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = NamedTempFile::new_in(parent)
        .with_context(|| format!("create temporary file in {}", parent.display()))?;
    write(&mut temp).with_context(|| format!("encode {}", path.display()))?;
    temp.persist_noclobber(path)
        .with_context(|| format!("create output {}", path.display()))?;
    Ok(())
}

fn encode_image(rgba: RgbaImage, format: ImageFormat, file: &mut NamedTempFile) -> Result<()> {
    let image = DynamicImage::ImageRgba8(rgba);
    let image = if format == ImageFormat::Jpeg {
        DynamicImage::ImageRgb8(image.into_rgb8())
    } else {
        image
    };
    let mut writer = BufWriter::new(file.as_file_mut());
    image.write_to(&mut writer, format)?;
    writer.flush()?;
    Ok(())
}

fn encode_ffmpeg(executable: &Path, rgba: RgbaImage, codec: &str, output: &Path) -> Result<()> {
    let mut png = tempfile::tempfile()?;
    use std::io::Seek;
    DynamicImage::ImageRgba8(rgba).write_to(&mut png, ImageFormat::Png)?;
    png.rewind()?;
    let status = Command::new(executable)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "image2pipe",
            "-c:v",
            "png",
            "-i",
            "pipe:0",
            "-frames:v",
            "1",
            "-threads",
            "1",
            "-c:v",
            codec,
            "-f",
            "image2",
            "-update",
            "1",
        ])
        .arg(output)
        .stdin(Stdio::from(png))
        .stdout(Stdio::null())
        .status()
        .with_context(|| format!("run FFmpeg at {}", executable.display()))?;
    if !status.success() {
        bail!("FFmpeg exited with {status}");
    }
    anyhow::ensure!(
        std::fs::metadata(output)?.len() > 0,
        "FFmpeg produced no image"
    );
    Ok(())
}
