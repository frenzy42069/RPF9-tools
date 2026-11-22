use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
    process::Command,
};

use anyhow::{bail, ensure, Context, Result};
use clap::ValueEnum;
use tempfile::NamedTempFile;

use crate::assets::Asset;

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum VideoFormat {
    Bk2,
    Mp4,
    Mkv,
    Webm,
}

impl VideoFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Bk2 => "bk2",
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::Webm => "webm",
        }
    }
}

pub(crate) fn export(
    asset: &Asset,
    output: &Path,
    format: VideoFormat,
    ffmpeg: &Path,
) -> Result<()> {
    let parent = output.parent().context("output path has no parent")?;
    let mut result = NamedTempFile::new_in(parent)?;
    if matches!(format, VideoFormat::Bk2) {
        copy_bink(asset, result.as_file_mut())?;
        result.as_file_mut().flush()?;
    } else {
        let clip = asset
            .offset
            .map(|_| -> Result<_> {
                let mut clip = tempfile::Builder::new().suffix(".bk2").tempfile()?;
                copy_bink(asset, clip.as_file_mut())?;
                clip.as_file_mut().flush()?;
                Ok(clip)
            })
            .transpose()?;
        let input = clip
            .as_ref()
            .map_or(asset.source.as_path(), |file| file.path());
        let (container, video, audio) = match format {
            VideoFormat::Mp4 => ("mp4", "libx264", "aac"),
            VideoFormat::Mkv => ("matroska", "libx264", "aac"),
            VideoFormat::Webm => ("webm", "libvpx-vp9", "libopus"),
            VideoFormat::Bk2 => unreachable!(),
        };
        let mut command = Command::new(ffmpeg);
        command
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
            .arg(input)
            .args([
                "-map", "0:v:0", "-map", "0:a?", "-c:v", video, "-pix_fmt", "yuv420p", "-c:a",
                audio,
            ]);
        if matches!(format, VideoFormat::Mp4) {
            command.args(["-movflags", "+faststart"]);
        }
        let process = command
            .args(["-f", container])
            .arg(result.path())
            .output()
            .with_context(|| format!("run FFmpeg at {}", ffmpeg.display()))?;
        if !process.status.success() {
            let hint = if is_bink(input)? {
                " For Xbox Bink2, use --ffmpeg=/path/to/a/Bink2-capable/ffmpeg, or --format bk2 to copy the clip."
            } else {
                ""
            };
            bail!(
                "FFmpeg failed: {}{hint}",
                String::from_utf8_lossy(&process.stderr).trim()
            );
        }
        ensure!(
            result.as_file().metadata()?.len() > 0,
            "FFmpeg produced no video"
        );
    }
    result
        .persist_noclobber(output)
        .with_context(|| format!("create output {}", output.display()))?;
    Ok(())
}

fn is_bink(input: &Path) -> Result<bool> {
    let mut head = [0; 3];
    let mut input = File::open(input)?;
    Ok(input.read_exact(&mut head).is_ok() && matches!(&head, b"KB2" | b"BIK"))
}

fn copy_bink(asset: &Asset, output: &mut impl Write) -> Result<()> {
    let mut input = File::open(&asset.source)?;
    let start = asset.offset.unwrap_or(0);
    input.seek(SeekFrom::Start(start))?;
    let mut head = [0; 8];
    input.read_exact(&mut head)?;
    ensure!(
        matches!(&head[..3], b"KB2" | b"BIK"),
        "expected a Bink clip"
    );
    let declared = u64::from(u32::from_le_bytes(head[4..8].try_into()?)) + 8;
    let length = asset.bytes.unwrap_or(declared);
    ensure!(length == declared && length > 8, "invalid Bink clip size");
    output.write_all(&head)?;
    ensure!(
        io::copy(&mut input.take(length - 8), output)? == length - 8,
        "truncated Bink clip"
    );
    Ok(())
}
