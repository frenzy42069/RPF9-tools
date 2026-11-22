use std::{
    fs::{self, File},
    io::{self, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Output},
};

use rpf9_convert::{LocalizationFormat, ModelFormat, TextureFormat};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::range::StreamRecord;

#[derive(Clone, Debug, Default)]
pub(crate) struct Options {
    pub ffmpeg: Option<PathBuf>,
    pub audio_extractor: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(crate) enum Dir {
    Pack,
    Dictionary { offset: u64 },
    Audio { offset: u64 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum FileKind {
    Texture {
        offset: Option<u64>,
        entry: usize,
    },
    Model {
        offset: u64,
    },
    Video {
        offset: u64,
        stored: u64,
    },
    Localization {
        offset: u64,
    },
    Audio {
        offset: u64,
        track: usize,
        occurrence: usize,
        extension: String,
        size: u64,
    },
}

impl FileKind {
    pub(crate) fn listed_size(&self) -> u64 {
        match self {
            Self::Audio { size, .. } => *size,
            // Some players reject empty files before open can start the transcode.
            Self::Video { stored, .. } => *stored,
            _ => 0,
        }
    }
}

pub(crate) enum Projection {
    Dir(Dir),
    File(FileKind),
}

pub(crate) fn key(kind: &FileKind, options: &Options) -> String {
    format!("readable-v1:{kind:?}:{options:?}")
}

pub(crate) fn project(
    _path: &Path,
    entry: &StreamRecord,
    options: &Options,
) -> Option<(String, Projection)> {
    let offset = entry.offset;
    let stem = entry
        .stem
        .clone()
        .unwrap_or_else(|| format!("{offset:08x}"));
    let file = |kind| Projection::File(kind);
    let dir = |kind| Projection::Dir(kind);
    Some(match entry.ext.as_str() {
        "awc" if entry.raw => (format!("{stem}.tracks"), dir(Dir::Audio { offset })),
        "bk2" if entry.raw && options.ffmpeg.is_some() => (
            format!("{stem}.mp4"),
            file(FileKind::Video {
                offset,
                stored: entry.stored,
            }),
        ),
        _ if entry.dictionary => (format!("{stem}.textures"), dir(Dir::Dictionary { offset })),
        _ if entry.localization => (
            format!("{stem}.jsonl"),
            file(FileKind::Localization { offset }),
        ),
        _ if entry.stem.is_some() => (format!("{stem}.glb"), file(FileKind::Model { offset })),
        _ => return None,
    })
}

pub(crate) fn list(
    path: &Path,
    dir: &Dir,
    options: &Options,
) -> io::Result<Vec<(String, FileKind)>> {
    let (rows, offset) = match dir {
        Dir::Audio { offset } => return list_audio(path, *offset, options),
        Dir::Pack => (rpf9_convert::list_textures(path), None),
        Dir::Dictionary { offset } => (
            rpf9_convert::list_texture_dictionaries(path, Some(*offset), 0, 1, 1),
            Some(*offset),
        ),
    };
    Ok(rows
        .map_err(io::Error::other)?
        .into_iter()
        .filter_map(|row| {
            let entry = usize::try_from(row["index"].as_u64()?).ok()?;
            if !is_supported_texture(&row) {
                return None;
            }
            let stem = row["name"]
                .as_str()
                .map(safe_stem)
                .filter(|s| !s.is_empty());
            let name = match stem {
                Some(stem) => format!("{stem}-{entry:06}.png"),
                None => format!("{entry:06}.png"),
            };
            Some((name, FileKind::Texture { offset, entry }))
        })
        .collect())
}

fn is_supported_texture(row: &Value) -> bool {
    matches!(
        row["format"].as_u64(),
        Some(74 | 75 | 77 | 78 | 83 | 98 | 99)
    ) && ["width", "height"].iter().all(|field| {
        row[field]
            .as_u64()
            .is_some_and(|size| size > 0 && size.is_multiple_of(256))
    })
}

fn safe_stem(name: &str) -> String {
    name.chars()
        .take(100)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn generate(
    path: &Path,
    kind: &FileKind,
    options: &Options,
    output: &Path,
) -> io::Result<()> {
    match kind {
        FileKind::Texture {
            offset: Some(offset),
            entry,
        } => rpf9_convert::export_dictionary_texture(
            path,
            *offset,
            *entry,
            output,
            TextureFormat::Png,
            None,
        )
        .map_err(io::Error::other),
        FileKind::Texture {
            offset: None,
            entry,
        } => rpf9_convert::export_texture(path, Some(*entry), output, TextureFormat::Png, None)
            .map_err(io::Error::other),
        FileKind::Model { offset } => {
            rpf9_convert::export_model(path, Some(*offset), output, ModelFormat::Glb)
                .map_err(io::Error::other)
        }
        FileKind::Localization { offset } => rpf9_convert::export_localization(
            path,
            Some(*offset),
            output,
            LocalizationFormat::Jsonl,
        )
        .map_err(io::Error::other),
        FileKind::Video { offset, stored } => video(path, *offset, *stored, options, output),
        FileKind::Audio {
            offset,
            track,
            occurrence,
            extension,
            ..
        } => {
            let temp = tempfile::tempdir()?;
            let dest = temp.path().join("tracks");
            let process = audio_command(path, *offset, options)?
                .arg("--track")
                .arg(track.to_string())
                .arg("--occurrence")
                .arg(occurrence.to_string())
                .arg("--out-dir")
                .arg(&dest)
                .output()?;
            success(process, "ADAT extraction")?;
            let basename = audio_name(*track, *occurrence, extension);
            let file = dest.join(format!("{offset:09X}")).join(basename);
            fs::copy(file, output)?;
            Ok(())
        }
    }
}

fn audio_name(track: usize, occurrence: usize, extension: &str) -> String {
    if occurrence == 1 {
        format!("{track:05}-55.{extension}")
    } else {
        format!("{track:05}-55-{occurrence:02}.{extension}")
    }
}

fn audio_command(source: &Path, offset: u64, options: &Options) -> io::Result<Command> {
    let executable = match &options.audio_extractor {
        Some(path) => path.clone(),
        None => {
            let name = "rpf9-extract-audio";
            let sibling = std::env::current_exe()?.with_file_name(name);
            if sibling.is_file() {
                sibling
            } else {
                PathBuf::from(name)
            }
        }
    };
    let mut command = Command::new(executable);
    command
        .arg(std::path::absolute(source)?)
        .arg("--bank")
        .arg(format!("{offset:#x}"));
    Ok(command)
}

fn list_audio(path: &Path, offset: u64, options: &Options) -> io::Result<Vec<(String, FileKind)>> {
    let result = audio_command(path, offset, options)?
        .arg("--list")
        .output()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("cannot run ADAT helper (set --audio-extractor): {error}"),
            )
        })?;
    let result = success(result, "ADAT listing")?;
    let lines = std::str::from_utf8(&result.stdout).map_err(io::Error::other)?;
    if !lines.starts_with("bank\tbank_flags\tentry\t") {
        return Err(io::Error::other(
            "rebuild rpf9-extract-audio with --list support",
        ));
    }
    Ok(lines
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            let track: usize = fields.get(2)?.parse().ok()?;
            let occurrence: usize = fields.get(5)?.parse().ok()?;
            if *fields.get(4)? != "0x55" {
                return None;
            }
            let size: u64 = fields.get(14)?.parse().ok()?;
            let extension = match *fields.get(13)? {
                "opus" => "opus",
                "xma2" => "xma",
                _ => return None,
            };
            Some((
                audio_name(track, occurrence, extension),
                FileKind::Audio {
                    offset,
                    track,
                    occurrence,
                    extension: extension.into(),
                    size,
                },
            ))
        })
        .collect())
}

fn video(
    path: &Path,
    offset: u64,
    stored: u64,
    options: &Options,
    output: &Path,
) -> io::Result<()> {
    let ffmpeg = options
        .ffmpeg
        .as_ref()
        .ok_or_else(|| io::Error::other("configure a Bink2-capable FFmpeg"))?;
    let mut source = File::open(path)?;
    source.seek(SeekFrom::Start(offset))?;
    let mut clip = tempfile::Builder::new().suffix(".bk2").tempfile()?;
    use std::io::Read;
    if io::copy(&mut source.take(stored), &mut clip)? != stored {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated Bink clip",
        ));
    }
    let result = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-n", "-i"])
        .arg(clip.path())
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a?",
            "-c:v",
            "libx264",
            "-vf",
            "scale=ceil(iw/2)*2:ceil(ih/2)*2",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ])
        .arg(output)
        .output()?;
    success(result, "Bink2 conversion (requires a Bink2-capable FFmpeg)")?;
    if output.metadata()?.len() == 0 {
        return Err(io::Error::other("Bink2 conversion produced an empty MP4"));
    }
    Ok(())
}

fn success(output: Output, context: &str) -> io::Result<Output> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(io::Error::other(format!(
            "{context}: {}",
            String::from_utf8_lossy(&output.stderr).trim(),
        )))
    }
}
