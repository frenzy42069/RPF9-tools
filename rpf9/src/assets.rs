use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{bail, ensure, Context, Result};
use clap::ValueEnum;
use memmap2::Mmap;
use rpf9_core::{kinds::Kind as RawKind, media};
use serde::Serialize;
use serde_json::Value;

use crate::Selection;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    Model,
    Texture,
    Audio,
    Video,
    Localization,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Texture => "texture",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Localization => "localization",
        }
    }
}

#[derive(Serialize)]
pub(crate) struct Asset {
    pub kind: Kind,
    pub source: PathBuf,
    pub name: String,
    pub offset: Option<u64>,
    pub entry: Option<usize>,
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

impl Asset {
    fn new(selection: &Selection, name: String) -> Self {
        Self {
            kind: selection.kind,
            source: selection.input.clone(),
            name,
            offset: selection.offset,
            entry: selection.entry,
            bytes: None,
            metadata: Value::Null,
        }
    }

    pub fn output_name(&self, extension: Option<&str>) -> String {
        let basename: String = self
            .name
            .chars()
            .take(96)
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        let mut name = format!("{}-{basename}", self.kind.label());
        if let Some(offset) = self.offset {
            name.push_str(&format!("-{offset:x}"));
        }
        if let Some(entry) = self.entry {
            name.push_str(&format!("-{entry}"));
        }
        if let Some(extension) = extension {
            name.push('.');
            name.push_str(extension);
        }
        name
    }
}

pub(crate) fn find(selection: &Selection) -> Result<Vec<Asset>> {
    let mut signature = [0; 4];
    let mut file = File::open(&selection.input)
        .with_context(|| format!("open {}", selection.input.display()))?;
    match file.read_exact(&mut signature) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {}
        Err(error) => return Err(error.into()),
    }
    let archive = signature == *b"9FPR";
    if selection.entry.is_some() && selection.kind != Kind::Texture {
        bail!("--entry applies to textures");
    }
    if selection.kind == Kind::Texture
        && archive
        && selection.entry.is_some()
        && selection.offset.is_none()
    {
        bail!("--entry requires a dictionary --offset when selecting a texture in an archive");
    }
    if selection.kind == Kind::Texture && !archive && selection.offset.is_some() {
        bail!("--offset applies to texture dictionaries in an RPF9 archive");
    }
    let limit = selection.limit.map_or_else(
        || if selection.all { usize::MAX } else { 100 },
        |limit| limit as usize,
    );
    let find_limit = if selection.name.is_some() {
        usize::MAX
    } else {
        limit
    };
    let scan = if selection.all {
        u64::MAX
    } else {
        selection.scan_bytes
    };
    let mut entries = match selection.kind {
        Kind::Texture if signature == *b"RDS5" || archive => {
            let textures = if archive {
                rpf9_convert::list_texture_dictionaries(
                    &selection.input,
                    selection.offset,
                    selection.start,
                    scan,
                    find_limit,
                )?
            } else {
                rpf9_convert::list_textures(&selection.input)?
            };
            textures
                .into_iter()
                .filter_map(|info| {
                    let index = usize::try_from(info["index"].as_u64()?).ok()?;
                    if selection.entry.is_some_and(|requested| requested != index) {
                        return None;
                    }
                    let name = info["name"].as_str().map_or_else(
                        || format!("{}_{index}", stem(&selection.input)),
                        str::to_owned,
                    );
                    let mut entry = Asset::new(selection, name);
                    entry.offset = info["offset"].as_u64();
                    entry.entry = Some(index);
                    entry.metadata = info;
                    Some(entry)
                })
                .collect()
        }
        Kind::Model | Kind::Localization
            if archive && (selection.offset.is_none() || selection.kind == Kind::Model) =>
        {
            let list = if selection.kind == Kind::Model {
                match selection.offset {
                    Some(offset) => vec![rpf9_convert::inspect_model(&selection.input, offset)?],
                    None => rpf9_convert::list_models(
                        &selection.input,
                        selection.start,
                        scan,
                        find_limit,
                    )?,
                }
            } else {
                rpf9_convert::list_localization(
                    &selection.input,
                    selection.start,
                    scan,
                    find_limit,
                )?
            };
            list.into_iter()
                .map(|metadata| {
                    let offset = metadata["offset"]
                        .as_u64()
                        .context("missing offset in catalog")?;
                    let name = metadata["name"].as_str().map_or_else(
                        || format!("{}_{offset:x}", stem(&selection.input)),
                        str::to_owned,
                    );
                    let mut asset = Asset::new(selection, name);
                    asset.offset = Some(offset);
                    asset.metadata = metadata;
                    Ok(asset)
                })
                .collect::<Result<_>>()?
        }
        Kind::Audio | Kind::Video if archive || selection.offset.is_some() => {
            media_entries(selection, &file, scan, find_limit)?
        }
        _ => vec![Asset::new(selection, stem(&selection.input))],
    };
    if let Some(name) = &selection.name {
        let query = name.to_lowercase();
        entries.retain(|asset| asset.name.to_lowercase().contains(&query));
    }
    entries.truncate(limit);
    Ok(entries)
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn media_entries(
    selection: &Selection,
    file: &File,
    scan: u64,
    limit: usize,
) -> Result<Vec<Asset>> {
    let start = selection.offset.unwrap_or(selection.start);
    ensure!(
        start.is_multiple_of(16),
        "archive scan start must be 16-byte aligned"
    );
    ensure!(scan > 0, "--scan-bytes must be positive");
    let start = usize::try_from(start)?;
    // The read-only map is used while the source archive is stable.
    let map = unsafe { Mmap::map(file)? };
    let end = if selection.offset.is_some() {
        start.saturating_add(16)
    } else {
        start.saturating_add(usize::try_from(scan).unwrap_or(usize::MAX))
    }
    .min(map.len());
    let kind = match selection.kind {
        Kind::Video => RawKind::Movie,
        Kind::Audio => RawKind::Adat,
        _ => unreachable!("media discovery only handles audio and video"),
    };
    let mut items = Vec::new();
    for lower in (start..end).step_by(1024 * 1024) {
        let upper = lower.saturating_add(1024 * 1024).min(end);
        for stream in media::carve_media(&map, lower..upper, &[]) {
            if stream.kind != kind || stream.stored_len > map.len() - stream.at {
                continue;
            }
            let mut asset = Asset::new(
                selection,
                format!("{}_{:x}", stem(&selection.input), stream.at),
            );
            asset.offset = Some(stream.at as u64);
            asset.bytes = Some(stream.stored_len as u64);
            items.push(asset);
            if items.len() == limit {
                return Ok(items);
            }
        }
    }
    if selection.offset.is_some() {
        ensure!(
            !items.is_empty(),
            "no matching media at the requested offset"
        );
    }
    Ok(items)
}
