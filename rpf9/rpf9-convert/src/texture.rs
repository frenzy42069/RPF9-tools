use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use flate2::read::ZlibDecoder;
use image::RgbaImage;

pub(crate) fn load(path: &Path, entry: Option<usize>) -> Result<RgbaImage> {
    let mut signature = [0; 4];
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let signature_len = file
        .read(&mut signature)
        .with_context(|| format!("reading {}", path.display()))?;

    if signature_len == signature.len() && &signature == b"RDS5" {
        return load_rds5(&mut file, entry);
    }
    if signature_len == signature.len() && matches!(&signature, b"ETPF" | b"TTPF") {
        bail!("TTPF texture archives are not supported yet");
    }
    if entry.is_some() {
        bail!("a texture entry can only be selected from a texture archive");
    }

    image::open(path)
        .with_context(|| format!("decoding texture {}", path.display()))
        .map(|image| image.into_rgba8())
}

fn load_rds5(file: &mut File, entry: Option<usize>) -> Result<RgbaImage> {
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0; 24];
    file.read_exact(&mut header)
        .context("reading RDS5 header")?;
    let word = |bytes: &[u8]| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let chunk_count = word(&header[4..8]) as usize;
    let metadata_offset = u64::from(word(&header[16..20])) * 256;
    ensure!(
        (2..=1_000_000).contains(&chunk_count),
        "invalid RDS5 chunk count"
    );
    let table_size = (chunk_count - 1)
        .checked_mul(8)
        .context("RDS5 table size overflow")?;
    ensure!(
        24 + table_size as u64 <= metadata_offset,
        "invalid RDS5 metadata offset"
    );
    let mut table = vec![0; table_size];
    file.read_exact(&mut table)
        .context("reading RDS5 chunk table")?;
    file.seek(SeekFrom::Start(metadata_offset))?;
    let mut metadata = [0; 16];
    file.read_exact(&mut metadata)
        .context("reading ETPF header")?;
    ensure!(&metadata[0..4] == b"ETPF", "missing ETPF metadata");
    let count = word(&metadata[8..12]) as usize;
    let index = entry.unwrap_or(0);
    ensure!(
        index < count,
        "texture entry index {index} is out of range (count {count})"
    );
    let record_offset = metadata_offset
        .checked_add(16 + (index as u64) * 16)
        .context("ETPF record offset overflow")?;
    file.seek(SeekFrom::Start(record_offset))?;
    let mut record = [0; 16];
    file.read_exact(&mut record)
        .context("reading ETPF texture record")?;
    let dimensions = word(&record[0..4]);
    let width = (dimensions & 0x3fff) as usize + 1;
    let height = ((dimensions >> 14) & 0x3fff) as usize + 1;
    let properties = word(&record[4..8]);
    let format = (properties >> 3) & 0x7f;
    let small_tiles = properties & 0x0100_0000 != 0;
    ensure!(
        matches!(format, 77 | 78 | 98 | 99),
        "unsupported DXGI texture format {format}"
    );
    ensure!(
        width >= 256 && height >= 256 && width & 255 == 0 && height & 255 == 0,
        "unsupported tiled texture dimensions {width}x{height}"
    );
    let raw_len = width
        .checked_mul(height)
        .context("texture dimensions overflow")?;
    ensure!(raw_len <= 256 * 1024 * 1024, "texture is too large");
    let virtual_start = u64::from(word(&record[8..12]) & 0x000f_ffff)
        .checked_mul(4096)
        .and_then(|v| v.checked_sub(4096))
        .context("invalid texture address")?;
    let mut raw = Vec::with_capacity(raw_len);
    for descriptors in table.as_chunks::<8>().0.windows(2) {
        let chunk = word(&descriptors[0][..4]);
        let chunk_virtual = u64::from(word(&descriptors[0][4..8]) & 0x000f_ffff) * 16384;
        let next_virtual = u64::from(word(&descriptors[1][4..8]) & 0x000f_ffff) * 16384;
        if next_virtual <= virtual_start || chunk_virtual >= virtual_start + raw_len as u64 {
            continue;
        }
        ensure!(
            (chunk >> 26) & 15 == 8,
            "unsupported RDS5 chunk codec {}",
            (chunk >> 26) & 15
        );
        let start = u64::from(chunk & 0x03ff_ffff) << 8;
        let end = u64::from(word(&descriptors[1][..4]) & 0x03ff_ffff) << 8;
        ensure!(
            end > start && end <= file.metadata()?.len() && next_virtual > chunk_virtual,
            "invalid RDS5 chunk bounds"
        );
        let decoded_len =
            usize::try_from(next_virtual - chunk_virtual).context("chunk too large")?;
        ensure!(decoded_len <= 256 * 1024 * 1024, "RDS5 chunk is too large");
        file.seek(SeekFrom::Start(start))?;
        let mut decoded = Vec::new();
        ZlibDecoder::new((&mut *file).take(end - start))
            .take(decoded_len as u64 + 1)
            .read_to_end(&mut decoded)
            .context("decompressing RDS5 texture")?;
        ensure!(decoded.len() == decoded_len, "unexpected RDS5 chunk size");
        let from = virtual_start.saturating_sub(chunk_virtual) as usize;
        let to = ((virtual_start + raw_len as u64).min(next_virtual) - chunk_virtual) as usize;
        ensure!(
            chunk_virtual + from as u64 == virtual_start + raw.len() as u64,
            "RDS5 texture chunks are discontinuous"
        );
        raw.extend_from_slice(&decoded[from..to]);
    }
    ensure!(raw.len() == raw_len, "texture data is incomplete");
    let blocks_wide = width / 4;
    let blocks_high = height / 4;
    let mut linear = vec![0; raw.len()];
    for y in 0..blocks_high {
        for x in 0..blocks_wide {
            let tile_width = if small_tiles { 16 } else { 64 };
            let tile = (y / tile_width) * (blocks_wide / tile_width) + x / tile_width;
            let tx = x % tile_width;
            let ty = y % tile_width;
            let mut inside = 0;
            let small_bits = [
                (ty, 0),
                (ty, 1),
                (tx, 0),
                (tx, 1),
                (ty, 2),
                (tx, 2),
                (ty, 3),
                (tx, 3),
            ];
            let large_bits = [
                (tx, 0),
                (ty, 0),
                (tx, 1),
                (ty, 1),
                (ty, 2),
                (tx, 2),
                (ty, 3),
                (tx, 3),
                (ty, 4),
                (tx, 4),
                (ty, 5),
                (tx, 5),
            ];
            let bits = if small_tiles {
                &small_bits[..]
            } else {
                &large_bits[..]
            };
            for (output_bit, &(axis, bit)) in bits.iter().enumerate() {
                inside |= ((axis >> bit) & 1) << output_bit;
            }
            let source = (tile * tile_width * tile_width + inside) * 16;
            let destination = (y * blocks_wide + x) * 16;
            linear[destination..destination + 16].copy_from_slice(&raw[source..source + 16]);
        }
    }
    let mut pixels = vec![0; width * height];
    let result = if matches!(format, 77 | 78) {
        texture2ddecoder::decode_bc3(&linear, width, height, &mut pixels)
    } else {
        texture2ddecoder::decode_bc7(&linear, width, height, &mut pixels)
    };
    result.map_err(|error| anyhow::anyhow!("decoding block texture: {error}"))?;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for pixel in pixels {
        let [b, g, r, a] = pixel.to_le_bytes();
        rgba.extend_from_slice(&[r, g, b, a]);
    }
    RgbaImage::from_raw(width as u32, height as u32, rgba)
        .context("invalid decoded texture dimensions")
}
