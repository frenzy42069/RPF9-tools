use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use flate2::read::ZlibDecoder;
use image::RgbaImage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextureInfo {
    pub index: usize,
    pub width: u32,
    pub height: u32,
    pub format: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DictionaryTextureInfo {
    pub index: usize,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub format: u32,
}

struct DictionaryEntry {
    info: DictionaryTextureInfo,
    graphics_offset: usize,
    small_tiles: bool,
}

fn word(bytes: &[u8], offset: usize) -> Result<u32> {
    let data: [u8; 4] = bytes
        .get(offset..offset + 4)
        .context("truncated texture dictionary")?
        .try_into()?;
    Ok(u32::from_le_bytes(data))
}

fn pointer(bytes: &[u8], offset: usize, base: u32) -> Result<usize> {
    ensure!(word(bytes, offset + 4)? == 0, "invalid texture pointer");
    let value = word(bytes, offset)?
        .checked_sub(base)
        .context("invalid texture pointer")?;
    ensure!(value < 0x1000_0000, "invalid texture pointer");
    Ok(value as usize)
}

fn read_dictionary_page(reader: &mut BufReader<File>, limit: u64) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    flate2::bufread::ZlibDecoder::new(reader)
        .take(limit + 1)
        .read_to_end(&mut output)
        .context("decompressing texture dictionary")?;
    ensure!(
        output.len() as u64 <= limit,
        "texture dictionary page exceeds size limit"
    );
    Ok(output)
}

fn dictionary_entries(cpu: &[u8]) -> Result<Vec<DictionaryEntry>> {
    let count = (word(cpu, 0x38)? & 0xffff) as usize;
    ensure!(
        (1..=4096).contains(&count),
        "invalid texture dictionary count"
    );
    let table = pointer(cpu, 0x30, 0x5000_0000)?;
    ensure!(
        table
            .checked_add(count * 8)
            .is_some_and(|end| end <= cpu.len()),
        "texture dictionary table exceeds page"
    );
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let descriptor = pointer(cpu, table + index * 8, 0x5000_0000)?;
        let name_offset = pointer(cpu, descriptor + 0x30, 0x5000_0000)?
            .checked_add(8)
            .context("texture name offset overflow")?;
        let name_bytes = cpu
            .get(name_offset..)
            .context("invalid texture name pointer")?;
        let end = name_bytes
            .iter()
            .position(|&byte| byte == 0)
            .context("unterminated texture name")?;
        ensure!(end > 0 && end <= 255, "invalid texture name");
        let name = std::str::from_utf8(&name_bytes[..end]).context("invalid texture name")?;
        let dimensions = word(cpu, descriptor + 0x38)?;
        let width = dimensions & 0xffff;
        let height = dimensions >> 16;
        ensure!(width > 0 && height > 0, "invalid texture dimensions");
        let format = word(cpu, descriptor + 0x3c)? >> 24;
        let graphics_offset = pointer(cpu, descriptor + 0x18, 0x6000_0000)?;
        let small_tiles = word(cpu, descriptor + 0x40)? & 0x8000_0000 != 0;
        entries.push(DictionaryEntry {
            info: DictionaryTextureInfo {
                index,
                name: name.to_owned(),
                width,
                height,
                format,
            },
            graphics_offset,
            small_tiles,
        });
    }
    Ok(entries)
}

pub(crate) fn is_dictionary(page: &[u8]) -> bool {
    page.get(..8) == Some(&[0; 8])
        && pointer(page, 8, 0x5000_0000).is_ok()
        && dictionary_entries(page).is_ok()
}

pub(crate) fn list_dictionary(path: &Path, offset: u64) -> Result<Vec<DictionaryTextureInfo>> {
    let mut reader = BufReader::new(File::open(path)?);
    reader.seek(SeekFrom::Start(offset))?;
    let cpu = read_dictionary_page(&mut reader, 64 * 1024 * 1024)?;
    Ok(dictionary_entries(&cpu)?
        .into_iter()
        .map(|entry| entry.info)
        .collect())
}

/// Find texture dictionaries in a bounded region of an archive.
pub(crate) fn list_dictionaries(
    path: &Path,
    start: u64,
    scan_bytes: u64,
    limit: usize,
) -> Result<Vec<(u64, Vec<DictionaryTextureInfo>)>> {
    ensure!(
        scan_bytes > 0 && limit > 0,
        "scan length and limit must be positive"
    );
    let file = File::open(path)?;
    let end = start.saturating_add(scan_bytes).min(file.metadata()?.len());
    let mut scanner = BufReader::new(file);
    let mut candidate_reader = BufReader::new(File::open(path)?);
    let mut chunk = vec![0u8; 1024 * 1024];
    let mut position = start.checked_add(15).context("scan offset overflow")? & !15;
    let mut dictionaries = Vec::new();
    while position < end && dictionaries.len() < limit {
        scanner.seek(SeekFrom::Start(position))?;
        let count = (end - position).min(chunk.len() as u64) as usize;
        scanner.read_exact(&mut chunk[..count])?;
        for relative in (0..count.saturating_sub(1)).step_by(16) {
            let cmf = chunk[relative];
            let flg = chunk[relative + 1];
            if cmf != 0x78 || (u16::from(cmf) * 256 + u16::from(flg)) % 31 != 0 || flg & 0x20 != 0 {
                continue;
            }
            let offset = position + relative as u64;
            candidate_reader.seek(SeekFrom::Start(offset))?;
            let mut prefix = [0u8; 0x40];
            if flate2::bufread::ZlibDecoder::new(&mut candidate_reader)
                .read_exact(&mut prefix)
                .is_err()
            {
                continue;
            }
            if prefix[..8] != [0; 8]
                || pointer(&prefix, 8, 0x5000_0000).is_err()
                || pointer(&prefix, 0x30, 0x5000_0000).is_err()
                || !matches!(word(&prefix, 0x38), Ok(value) if (1..=4096).contains(&(value & 0xffff)))
            {
                continue;
            }
            candidate_reader.seek(SeekFrom::Start(offset))?;
            let Ok(cpu) = read_dictionary_page(&mut candidate_reader, 64 * 1024 * 1024) else {
                continue;
            };
            let Ok(entries) = dictionary_entries(&cpu) else {
                continue;
            };
            dictionaries.push((
                offset,
                entries.into_iter().map(|entry| entry.info).collect(),
            ));
            if dictionaries.len() == limit {
                break;
            }
        }
        position += count as u64;
    }
    Ok(dictionaries)
}

pub(crate) fn load_dictionary(path: &Path, offset: u64, index: usize) -> Result<RgbaImage> {
    let mut reader = BufReader::new(File::open(path)?);
    reader.seek(SeekFrom::Start(offset))?;
    let cpu = read_dictionary_page(&mut reader, 64 * 1024 * 1024)?;
    let entries = dictionary_entries(&cpu)?;
    let entry = entries
        .get(index)
        .context("texture dictionary entry is out of range")?;
    let width = entry.info.width as usize;
    let height = entry.info.height as usize;
    ensure!(
        matches!(entry.info.format, 74 | 75 | 77 | 78 | 83 | 98 | 99),
        "unsupported DXGI texture format {}",
        entry.info.format
    );
    ensure!(
        width >= 256 && height >= 256 && width.is_multiple_of(256) && height.is_multiple_of(256),
        "unsupported tiled texture dimensions {width}x{height}"
    );
    let len = width.checked_mul(height).context("texture size overflow")?;
    ensure!(len <= 256 * 1024 * 1024, "texture is too large");
    let position = reader.stream_position()?;
    let padding = (16 - position % 16) % 16;
    let mut pad = [0u8; 15];
    reader.read_exact(&mut pad[..padding as usize])?;
    ensure!(pad == [0; 15], "invalid texture dictionary padding");
    let graphics = read_dictionary_page(&mut reader, 256 * 1024 * 1024)?;
    let end = entry
        .graphics_offset
        .checked_add(len)
        .context("texture data size overflow")?;
    let raw = graphics
        .get(entry.graphics_offset..end)
        .context("texture data exceeds graphics page")?;
    decode_blocks(raw, width, height, entry.info.format, entry.small_tiles)
}

/// Read the ETPF directory without inflating texture chunks.
pub(crate) fn list(path: &Path) -> Result<Vec<TextureInfo>> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut header = [0; 24];
    file.read_exact(&mut header)
        .context("reading RDS5 header")?;
    ensure!(&header[..4] == b"RDS5", "expected an RDS5 texture archive");
    let metadata_offset = u64::from(u32::from_le_bytes(header[16..20].try_into()?)) * 256;
    ensure!(metadata_offset >= 24, "invalid RDS5 metadata offset");
    let file_size = file.metadata()?.len();
    ensure!(
        metadata_offset <= file_size.saturating_sub(16),
        "invalid RDS5 metadata offset"
    );
    file.seek(SeekFrom::Start(metadata_offset))?;
    let mut metadata = [0; 16];
    file.read_exact(&mut metadata)?;
    ensure!(&metadata[..4] == b"ETPF", "missing ETPF metadata");
    let count = u32::from_le_bytes(metadata[8..12].try_into()?) as usize;
    let table_bytes = (count as u64)
        .checked_mul(16)
        .context("ETPF table size overflow")?;
    ensure!(
        metadata_offset + 16 + table_bytes <= file_size,
        "ETPF texture table extends beyond the file"
    );
    ensure!(count <= 1_000_000, "too many texture entries");
    let mut textures = Vec::with_capacity(count);
    for index in 0..count {
        let mut record = [0; 16];
        file.read_exact(&mut record)
            .context("reading ETPF texture record")?;
        let dimensions = u32::from_le_bytes(record[..4].try_into()?);
        let properties = u32::from_le_bytes(record[4..8].try_into()?);
        textures.push(TextureInfo {
            index,
            width: (dimensions & 0x3fff) + 1,
            height: ((dimensions >> 14) & 0x3fff) + 1,
            format: (properties >> 3) & 0x7f,
        });
    }
    Ok(textures)
}

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
        matches!(format, 74 | 75 | 77 | 78 | 83 | 98 | 99),
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
        let codec = (chunk >> 26) & 15;
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
        match codec {
            0 => {
                ensure!(
                    end - start >= decoded_len as u64,
                    "truncated raw RDS5 chunk"
                );
                (&mut *file)
                    .take(decoded_len as u64)
                    .read_to_end(&mut decoded)?;
            }
            8 => {
                ZlibDecoder::new((&mut *file).take(end - start))
                    .take(decoded_len as u64 + 1)
                    .read_to_end(&mut decoded)
                    .context("decompressing RDS5 texture")?;
            }
            _ => bail!("unsupported RDS5 chunk codec {codec}"),
        }
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
    decode_blocks(&raw, width, height, format, small_tiles)
}

fn decode_blocks(
    raw: &[u8],
    width: usize,
    height: usize,
    format: u32,
    small_tiles: bool,
) -> Result<RgbaImage> {
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
    let mut pixels = vec![0xff00_0000; width * height];
    let result = match format {
        74 | 75 => texture2ddecoder::decode_bc2(&linear, width, height, &mut pixels),
        77 | 78 => texture2ddecoder::decode_bc3(&linear, width, height, &mut pixels),
        83 => texture2ddecoder::decode_bc5(&linear, width, height, &mut pixels),
        98 | 99 => texture2ddecoder::decode_bc7(&linear, width, height, &mut pixels),
        _ => unreachable!("format validated above"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, Write};

    #[test]
    fn finds_and_decodes_texture_dictionary_in_local_archive() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game/textures.rpf");
        if !path.is_file() {
            return;
        }
        let entries = list_dictionary(&path, 0x2010).unwrap();
        assert_eq!(entries.len(), 7);
        assert_eq!(entries[0].name, "pistol_glass_crack_decals");
        assert_eq!(
            (entries[0].width, entries[0].height, entries[0].format),
            (512, 512, 99)
        );
        let image = load_dictionary(&path, 0x2010, 0).unwrap();
        assert_eq!(image.dimensions(), (512, 512));
        assert!(image.pixels().any(|pixel| pixel.0[0] > 100));
        let found = list_dictionaries(&path, 0, 0x400000, 10).unwrap();
        assert!(found.iter().any(|(offset, _)| *offset == 0x2010));
        assert!(found.iter().any(|(offset, _)| *offset == 0x228010));
    }

    #[test]
    fn indexes_and_decodes_raw_bc2_without_reencoding_the_payload() {
        // 256x256 BC2 texture with opaque red blocks.
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let mut bytes = vec![0u8; 0x200 + 65536];
        bytes[..4].copy_from_slice(b"RDS5");
        bytes[4..8].copy_from_slice(&3u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
        bytes[24..28].copy_from_slice(&2u32.to_le_bytes());
        bytes[32..36].copy_from_slice(&258u32.to_le_bytes());
        bytes[36..40].copy_from_slice(&4u32.to_le_bytes());
        bytes[0x100..0x104].copy_from_slice(b"ETPF");
        bytes[0x108..0x10c].copy_from_slice(&1u32.to_le_bytes());
        let dimensions = 255u32 | (255 << 14);
        bytes[0x110..0x114].copy_from_slice(&dimensions.to_le_bytes());
        bytes[0x114..0x118].copy_from_slice(&(74u32 << 3 | 0x0100_0000).to_le_bytes());
        bytes[0x118..0x11c].copy_from_slice(&1u32.to_le_bytes());
        for block in bytes[0x200..].as_chunks_mut::<16>().0 {
            block[..8].fill(0xff);
            block[8..10].copy_from_slice(&0xf800u16.to_le_bytes());
            block[10..12].copy_from_slice(&0xf800u16.to_le_bytes());
        }
        file.write_all(&bytes).unwrap();
        file.rewind().unwrap();
        assert_eq!(
            list(file.path()).unwrap(),
            [TextureInfo {
                index: 0,
                width: 256,
                height: 256,
                format: 74
            }]
        );
        let image = load(file.path(), None).unwrap();
        assert_eq!(image.dimensions(), (256, 256));
        assert!(image.pixels().all(|pixel| pixel.0 == [255, 0, 0, 255]));

        // BC5 stores independent red and green blocks with implicit opaque alpha.
        bytes[0x114..0x118].copy_from_slice(&(83u32 << 3 | 0x0100_0000).to_le_bytes());
        for block in bytes[0x200..].as_chunks_mut::<16>().0 {
            block.fill(0);
            block[0] = 200;
            block[1] = 200;
            block[8] = 100;
            block[9] = 100;
        }
        file.rewind().unwrap();
        file.write_all(&bytes).unwrap();
        let image = load(file.path(), None).unwrap();
        assert!(image.pixels().all(|pixel| pixel.0 == [200, 100, 0, 255]));
    }

    #[test]
    fn rejects_texture_table_beyond_file() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let mut bytes = vec![0u8; 0x110];
        bytes[..4].copy_from_slice(b"RDS5");
        bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x100..0x104].copy_from_slice(b"ETPF");
        bytes[0x108..0x10c].copy_from_slice(&2u32.to_le_bytes());
        file.write_all(&bytes).unwrap();
        assert!(list(file.path())
            .unwrap_err()
            .to_string()
            .contains("beyond the file"));
    }
}
