use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io,
    path::Path,
};

use memmap2::Mmap;
use rpf9_core::{media, names, stream};
use serde::{Deserialize, Serialize};

const LOOKAHEAD: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StreamRecord {
    pub offset: u64,
    pub stored: u64,
    pub size: u64,
    pub ext: String,
    pub raw: bool,
    pub stem: Option<String>,
    pub dictionary: bool,
    pub localization: bool,
}

pub(crate) fn index(path: &Path, start: u64, len: u64) -> io::Result<Vec<StreamRecord>> {
    let map = File::open(path)?;
    let file_size = map.metadata()?.len();
    if start >= file_size || len == 0 {
        return Ok(Vec::new());
    }
    if !start.is_multiple_of(16) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unaligned archive range",
        ));
    }
    // The input is opened read-only and must remain stable while being inspected.
    let data = unsafe { Mmap::map(&map)? };
    let start = usize::try_from(start).map_err(io::Error::other)?;
    let end = start
        .saturating_add(usize::try_from(len).unwrap_or(usize::MAX))
        .min(data.len());
    let limited = &data[..end.saturating_add(LOOKAHEAD).min(data.len())];
    let mut stems = HashMap::new();
    let mut dictionaries = HashSet::new();
    let mut localization = HashSet::new();
    let mut streams = stream::walk_with(limited, start, end, |entry, page| {
        if let Some(stem) = names::embedded_stem(entry, page) {
            stems.insert(entry.at, stem);
        } else if matches!(
            entry.kind,
            rpf9_core::kinds::Kind::Resource | rpf9_core::kinds::Kind::Rsc9
        ) {
            if rpf9_convert::is_texture_dictionary(page) {
                dictionaries.insert(entry.at);
            } else if rpf9_convert::is_localization_table(page) {
                localization.insert(entry.at);
            }
        }
    });
    let covered: Vec<_> = streams
        .iter()
        .map(|entry| entry.at..entry.at + entry.stored_len)
        .collect();
    streams.extend(media::carve_media(&data, start..end, &covered));
    streams.sort_unstable_by_key(|stream| stream.at);
    Ok(streams
        .into_iter()
        .filter(|stream| stream.stored_len <= data.len() - stream.at)
        .map(|stream| {
            let raw = stream.kind.is_media();
            let size = if raw {
                stream.stored_len
            } else {
                stream.decoded_len
            };
            StreamRecord {
                offset: stream.at as u64,
                stored: stream.stored_len as u64,
                size: size as u64,
                ext: stream.kind.label().0.into(),
                raw,
                stem: stems.remove(&stream.at),
                dictionary: dictionaries.contains(&stream.at),
                localization: localization.contains(&stream.at),
            }
        })
        .collect())
}
