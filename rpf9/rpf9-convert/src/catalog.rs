//! Discover drawable streams within a bounded RPF byte range.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use anyhow::{Context, Result};
use flate2::{Decompress, FlushDecompress};
use serde_json::{json, Value};

use crate::drawable;

const BLOCK: usize = 1024 * 1024;
const PROBE: usize = 4096;

pub(crate) fn list(path: &Path, start: u64, scan_bytes: u64, limit: usize) -> Result<Vec<Value>> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let length = file.metadata()?.len();
    let end = start.saturating_add(scan_bytes).min(length);
    let mut cursor = start;
    let mut models = Vec::new();
    let mut buffer = vec![0; BLOCK + PROBE];
    while cursor < end && models.len() < limit {
        file.seek(SeekFrom::Start(cursor))?;
        let span = usize::try_from((end - cursor).min(BLOCK as u64))?;
        let read_size = usize::try_from((length - cursor).min((span + PROBE) as u64))?;
        file.read_exact(&mut buffer[..read_size])?;
        // RPF stream starts are aligned to 16-byte boundaries.
        let skip = ((16 - cursor % 16) % 16) as usize;
        for i in (skip..span).step_by(16) {
            if i + 2 > read_size
                || buffer[i] != 0x78
                || !matches!(buffer[i + 1], 0x01 | 0x5e | 0x9c | 0xda)
            {
                continue;
            }
            let mut prefix = [0; 0xb0];
            let mut decoder = Decompress::new(true);
            let _ = decoder.decompress(
                &buffer[i..read_size.min(i + PROBE)],
                &mut prefix,
                FlushDecompress::None,
            );
            if decoder.total_out() < prefix.len() as u64 || prefix[..8] != [0; 8] {
                continue;
            }
            let name_address = u64::from_le_bytes(prefix[0xa8..0xb0].try_into()?);
            if !(0x5000_0000..0x5400_0000).contains(&name_address) {
                continue;
            }
            let offset = cursor + i as u64;
            if let Ok((name, mesh)) = drawable::load_named(path, offset) {
                models.push(json!({
                    "name": name,
                    "offset": offset,
                    "vertices": mesh.positions.len(),
                    "triangles": mesh.indices.len() / 3,
                }));
                if models.len() == limit {
                    break;
                }
            }
        }
        cursor += span as u64;
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_convertible_drawables_in_the_requested_range() {
        let archive = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game/levels.rpf");
        if !archive.exists() {
            return;
        }
        let models = list(&archive, 0xf9c00000, 0x200000, 10).unwrap();
        let found = models
            .iter()
            .find(|item| item["offset"] == 0xf9c02010u64)
            .unwrap();
        assert_eq!(found["name"], "int_abd_1_gfa_rt_1_shl.#dr");
        assert_eq!(found["triangles"], 350);
        assert_eq!(list(&archive, 0xf9c02020, 0x1000, 10).unwrap().len(), 0);
        assert_eq!(list(&archive, 0xf9c00000, 0x200000, 1).unwrap().len(), 1);
    }
}
