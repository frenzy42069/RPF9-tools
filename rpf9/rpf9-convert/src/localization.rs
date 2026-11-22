use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{ensure, Context, Result};
use clap::ValueEnum;
use flate2::bufread::ZlibDecoder;
use serde_json::{json, Value};

const BASE: u64 = 0x5000_0000;
const MAX_RESOURCE: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Jsonl,
    Csv,
}

struct Record {
    key: u32,
    text: String,
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .context("truncated localization table")?
            .try_into()?,
    ))
}

fn u64_at(bytes: &[u8], offset: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        bytes
            .get(offset..offset + 8)
            .context("truncated localization table")?
            .try_into()?,
    ))
}

fn pointer(bytes: &[u8], offset: usize) -> Result<usize> {
    let value = u64_at(bytes, offset)?;
    let relative = value
        .checked_sub(BASE)
        .context("invalid localization pointer")?;
    ensure!(
        relative < MAX_RESOURCE as u64,
        "localization pointer is out of range"
    );
    Ok(relative as usize)
}

pub(crate) fn is_table(page: &[u8]) -> bool {
    records(page).is_ok()
}

fn records(bytes: &[u8]) -> Result<Vec<Record>> {
    ensure!(
        bytes.get(16..21) == Some(b"0DRPz"),
        "not a localization resource"
    );
    let desc = pointer(bytes, 0x38)?;
    let strings = pointer(bytes, desc + 8)?;
    let strings_size = u32_at(bytes, desc + 4)? as usize;
    let hashes = pointer(bytes, desc + 0x28)?;
    let hashes_size = u32_at(bytes, desc + 0x24)? as usize;
    let pool = pointer(bytes, desc + 0x38)?;
    let pool_size = u32_at(bytes, desc + 0x34)? as usize;
    ensure!(
        strings_size > 0 && strings_size.is_multiple_of(16) && hashes_size.is_multiple_of(16),
        "invalid localization table sizes"
    );
    let entries = bytes
        .get(
            strings
                ..strings
                    .checked_add(strings_size)
                    .context("table size overflow")?,
        )
        .context("truncated string index")?;
    let buckets = bytes
        .get(
            hashes
                ..hashes
                    .checked_add(hashes_size)
                    .context("table size overflow")?,
        )
        .context("truncated hash table")?;
    let text_pool = bytes
        .get(pool..pool.checked_add(pool_size).context("pool size overflow")?)
        .context("truncated text pool")?;
    let mut keys = vec![None; entries.len() / 16];
    for bucket in buckets.as_chunks::<16>().0 {
        if u32_at(bucket, 4)? != 0 {
            continue;
        }
        let value = u64_at(bucket, 8)?;
        if value & 0xff_ffff != 1 {
            continue;
        }
        let index = usize::try_from(value >> 24)?;
        if index >= keys.len() {
            continue;
        }
        ensure!(keys[index].is_none(), "duplicate localization index");
        keys[index] = Some(u32_at(bucket, 0)?);
    }
    let mut result = Vec::with_capacity(keys.len());
    for (entry, key) in entries.as_chunks::<16>().0.iter().zip(keys) {
        let key = key.context("missing localization key")?;
        ensure!(u64_at(entry, 0)? == 0, "unsupported localization entry");
        let value = u64_at(entry, 8)?;
        ensure!(
            value & 0xf_ffff == 4,
            "unsupported localization string pointer"
        );
        let offset = usize::try_from(value >> 20)?;
        let rest = text_pool
            .get(offset..)
            .context("localization string outside pool")?;
        let len = rest
            .iter()
            .position(|&byte| byte == 0)
            .context("unterminated localization string")?;
        let text =
            std::str::from_utf8(&rest[..len]).context("invalid UTF-8 localization string")?;
        result.push(Record {
            key,
            text: text.to_owned(),
        });
    }
    Ok(result)
}

fn page(reader: &mut BufReader<File>, bytes: &mut Vec<u8>) -> Result<usize> {
    let mut decoder = ZlibDecoder::new(reader);
    decoder
        .by_ref()
        .take((MAX_RESOURCE - bytes.len()) as u64 + 1)
        .read_to_end(bytes)?;
    let consumed = usize::try_from(decoder.total_in())?;
    ensure!(
        bytes.len() <= MAX_RESOURCE,
        "localization resource exceeds size limit"
    );
    Ok(consumed)
}

fn read_resource(file: &mut File, offset: u64) -> Result<Vec<u8>> {
    let mut reader = BufReader::new(file.try_clone()?);
    reader.seek(SeekFrom::Start(offset))?;
    let magic = reader.fill_buf()?;
    if magic.starts_with(b"\0\0\0\0\0\0\0\0") {
        let mut bytes = Vec::new();
        reader
            .take(MAX_RESOURCE as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= MAX_RESOURCE,
            "localization resource exceeds size limit"
        );
        return Ok(bytes);
    }
    let mut bytes = Vec::new();
    let mut position = offset;
    loop {
        let consumed =
            page(&mut reader, &mut bytes).context("decompressing localization resource")?;
        if records(&bytes).is_ok() {
            return Ok(bytes);
        }
        ensure!(
            consumed > 0 && bytes.len().is_multiple_of(65536),
            "incomplete or unsupported localization resource"
        );
        position = position
            .checked_add(consumed as u64)
            .context("stream position overflow")?;
        position = position
            .checked_add(15)
            .context("stream position overflow")?
            & !15;
        reader.seek(SeekFrom::Start(position))?;
        let magic = reader.fill_buf()?;
        ensure!(
            magic.first() == Some(&0x78),
            "localization continuation stream missing"
        );
    }
}

pub(crate) fn export(
    input: &Path,
    offset: Option<u64>,
    output: &mut impl Write,
    format: Format,
) -> Result<()> {
    let mut file = File::open(input)?;
    let decoded = read_resource(&mut file, offset.unwrap_or(0))?;
    let records = records(&decoded)?;
    if matches!(format, Format::Csv) {
        writeln!(output, "key,locale,text")?;
    }
    for record in records {
        let key = format!("0x{:08x}", record.key);
        match format {
            Format::Jsonl => {
                serde_json::to_writer(
                    &mut *output,
                    &json!({"key": key, "locale": null, "text": record.text}),
                )?;
                writeln!(output)?;
            }
            Format::Csv => writeln!(output, "{key},,\"{}\"", record.text.replace('"', "\"\""))?,
        }
    }
    Ok(())
}

pub(crate) fn list(input: &Path, start: u64, scan_bytes: u64, limit: usize) -> Result<Vec<Value>> {
    let mut file = File::open(input)?;
    let end = start.saturating_add(scan_bytes).min(file.metadata()?.len());
    if start >= end || limit == 0 {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    let mut position = start.checked_add(15).context("offset overflow")? & !15;
    let mut buffer = vec![0; 1024 * 1024];
    while position < end && found.len() < limit {
        file.seek(SeekFrom::Start(position))?;
        let count =
            file.read(&mut buffer[..usize::try_from((end - position).min(1024 * 1024))?])?;
        if count == 0 {
            break;
        }
        for index in (0..count.saturating_sub(1)).step_by(16) {
            if buffer[index] != 0x78 || !matches!(buffer[index + 1], 1 | 0x5e | 0x9c | 0xda) {
                continue;
            }
            let at = position + index as u64;
            let mut probe = [0_u8; 21];
            let mut decoder = ZlibDecoder::new(&buffer[index..count]);
            if decoder.read_exact(&mut probe).is_err() || &probe[16..21] != b"0DRPz" {
                continue;
            }
            if let Ok(data) = read_resource(&mut file, at) {
                if let Ok(items) = records(&data) {
                    found.push(json!({"offset": at, "count": items.len(), "locale": null}));
                    if found.len() >= limit {
                        break;
                    }
                }
            }
        }
        if count < buffer.len() {
            break;
        }
        position += count as u64;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_real_localization_table() -> Result<()> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game/data.rpf");
        if !path.exists() {
            return Ok(());
        }
        let mut jsonl = Vec::new();
        export(&path, Some(0x1219010), &mut jsonl, Format::Jsonl)?;
        let lines: Vec<Value> = std::str::from_utf8(&jsonl)?
            .lines()
            .map(serde_json::from_str)
            .collect::<serde_json::Result<_>>()?;
        assert_eq!(lines.len(), 656);
        assert_eq!(lines[0]["key"], "0xda16606e");
        assert_eq!(lines[0]["text"], "Watch out.");
        let mut csv = Vec::new();
        export(&path, Some(0x1219010), &mut csv, Format::Csv)?;
        assert!(
            std::str::from_utf8(&csv)?.starts_with("key,locale,text\n0xda16606e,,\"Watch out.\"")
        );
        let catalog = list(&path, 0x1219000, 0x1000, 1)?;
        assert_eq!(catalog[0]["offset"], 0x1219010_u64);
        assert_eq!(catalog[0]["count"], 656);
        Ok(())
    }
}
