mod audio;
mod catalog;
mod drawable;
mod localization;
mod mesh;
mod model_output;
mod texture;

use std::{
    io::{BufWriter, Write},
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use image::{DynamicImage, ImageFormat, RgbaImage};
use tempfile::NamedTempFile;

pub use audio::{export as export_audio, AudioOptions};
pub use localization::Format as LocalizationFormat;
pub use model_output::Format as ModelFormat;

pub fn is_texture_dictionary(page: &[u8]) -> bool {
    texture::is_dictionary(page)
}

pub fn is_localization_table(page: &[u8]) -> bool {
    localization::is_table(page)
}

pub fn inspect_model(input: &Path, offset: u64) -> Result<serde_json::Value> {
    let (name, mesh) = drawable::load_named(input, offset)?;
    Ok(serde_json::json!({
        "name": name,
        "offset": offset,
        "vertices": mesh.positions.len(),
        "triangles": mesh.indices.len() / 3,
    }))
}

pub fn list_textures(input: &Path) -> Result<Vec<serde_json::Value>> {
    Ok(texture::list(input)?.into_iter().map(|info| serde_json::json!({
        "index": info.index, "width": info.width, "height": info.height, "format": info.format,
    })).collect())
}

pub fn list_texture_dictionaries(
    input: &Path,
    offset: Option<u64>,
    start: u64,
    scan_bytes: u64,
    limit: usize,
) -> Result<Vec<serde_json::Value>> {
    let dictionaries = if let Some(offset) = offset {
        vec![(offset, texture::list_dictionary(input, offset)?)]
    } else {
        texture::list_dictionaries(input, start, scan_bytes, limit)?
    };
    Ok(dictionaries
        .into_iter()
        .flat_map(|(offset, entries)| {
            entries.into_iter().map(move |info| {
                serde_json::json!({
                    "offset": offset,
                    "index": info.index, "name": info.name, "width": info.width,
                    "height": info.height, "format": info.format,
                })
            })
        })
        .collect())
}

pub fn list_localization(
    input: &Path,
    start: u64,
    scan_bytes: u64,
    limit: usize,
) -> Result<Vec<serde_json::Value>> {
    localization::list(input, start, scan_bytes, limit)
}

pub fn export_localization(
    input: &Path,
    offset: Option<u64>,
    output: &Path,
    format: LocalizationFormat,
) -> Result<()> {
    write_output(output, |file| {
        let mut writer = BufWriter::new(file.as_file_mut());
        localization::export(input, offset, &mut writer, format)?;
        writer.flush()?;
        Ok(())
    })
}

pub fn list_models(
    input: &Path,
    start: u64,
    scan_bytes: u64,
    limit: usize,
) -> Result<Vec<serde_json::Value>> {
    catalog::list(input, start, scan_bytes, limit)
}

pub fn export_model(
    input: &Path,
    offset: Option<u64>,
    output: &Path,
    format: ModelFormat,
) -> Result<()> {
    let mesh = match offset {
        Some(offset) => drawable::load(input, offset),
        None => mesh::load(input),
    }
    .with_context(|| format!("read model {}", input.display()))?;
    write_output(output, |file| {
        let mut writer = BufWriter::new(file.as_file_mut());
        model_output::write(&mut writer, &mesh, format)?;
        writer.flush()?;
        Ok(())
    })
}

pub fn export_texture(
    input: &Path,
    entry: Option<usize>,
    output: &Path,
    format: TextureFormat,
    ffmpeg: Option<&Path>,
) -> Result<()> {
    let rgba =
        texture::load(input, entry).with_context(|| format!("read texture {}", input.display()))?;
    write_texture(rgba, output, format, ffmpeg)
}

pub fn export_dictionary_texture(
    input: &Path,
    offset: u64,
    entry: usize,
    output: &Path,
    format: TextureFormat,
    ffmpeg: Option<&Path>,
) -> Result<()> {
    let rgba = texture::load_dictionary(input, offset, entry)
        .with_context(|| format!("read texture dictionary at {offset:#x}"))?;
    write_texture(rgba, output, format, ffmpeg)
}

fn write_texture(
    rgba: RgbaImage,
    output: &Path,
    format: TextureFormat,
    ffmpeg: Option<&Path>,
) -> Result<()> {
    let (_, image_format, codec) = format.settings();
    write_output(output, |file| match ffmpeg {
        Some(executable) => encode_ffmpeg(executable, rgba, codec, file.path()),
        None => encode_image(rgba, image_format, file),
    })
}

#[derive(Clone, Copy, ValueEnum)]
pub enum TextureFormat {
    Png,
    #[value(alias = "jpg")]
    Jpeg,
    Webp,
    Bmp,
    Tga,
}

impl TextureFormat {
    pub fn extension(self) -> &'static str {
        self.settings().0
    }

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
