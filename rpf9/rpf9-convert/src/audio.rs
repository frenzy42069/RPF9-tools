//! Adapter for the ADAT extractor. Opus and XMA2 packets are retained by default.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Options for extracting ADAT banks from an RPF9 archive.
pub struct AudioOptions<'a> {
    pub input: &'a Path,
    pub output: &'a Path,
    /// An absolute byte offset selects one bank. `None` scans the archive.
    pub bank: Option<u64>,
    /// The extractor executable; defaults to `rpf9-extract-audio` on PATH.
    pub extractor: Option<&'a Path>,
    /// Also decode wrapped audio tracks into WAV using this FFmpeg executable.
    pub wav_ffmpeg: Option<&'a Path>,
}

/// Export supported tracks and a manifest into a new output directory.
///
/// Unknown or invalid payloads remain verbatim `.bin` files. The extractor
/// writes `manifest.tsv`, including each track's codec, status, and source range.
pub fn export(options: &AudioOptions<'_>) -> Result<()> {
    let extractor = options.extractor.map(Path::to_path_buf).unwrap_or_else(|| {
        let name = format!("rpf9-extract-audio{}", std::env::consts::EXE_SUFFIX);
        let beside_cli = std::env::current_exe()
            .ok()
            .map(|path| path.with_file_name(&name));
        beside_cli
            .filter(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from(name))
    });
    let mut command = Command::new(&extractor);
    command
        .arg(std::path::absolute(options.input)?)
        .arg("--out-dir")
        .arg(options.output);
    if let Some(bank) = options.bank {
        command.arg("--bank").arg(format!("0x{bank:x}"));
    }
    let status = command.status().with_context(|| {
        format!(
            "run ADAT extractor {} (build scripts/rpf9_extract_audio.c or provide its path)",
            Path::new(&extractor).display()
        )
    })?;
    if !status.success() {
        bail!("ADAT extractor exited with {status}; output may be incomplete");
    }
    if let Some(ffmpeg) = options.wav_ffmpeg {
        export_wav(options.output, ffmpeg)?;
    }
    Ok(())
}

fn export_wav(output: &Path, ffmpeg: &Path) -> Result<()> {
    for bank in fs::read_dir(output).with_context(|| format!("read {}", output.display()))? {
        let bank = bank?;
        if !bank.file_type()?.is_dir() {
            continue;
        }
        for entry in fs::read_dir(bank.path())? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            if !matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("opus" | "xma")
            ) {
                continue;
            }
            let output_path: PathBuf = path.with_extension("wav");
            let temporary = tempfile::NamedTempFile::new_in(bank.path())?;
            let result = Command::new(ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
                .arg(&path)
                .args(["-f", "wav"])
                .arg(temporary.path())
                .output()
                .with_context(|| format!("run FFmpeg at {}", ffmpeg.display()))?;
            if !result.status.success() {
                bail!(
                    "FFmpeg failed decoding {}: {}",
                    path.display(),
                    String::from_utf8_lossy(&result.stderr).trim()
                );
            }
            if temporary.as_file().metadata()?.len() == 0 {
                bail!("FFmpeg produced empty audio for {}", path.display());
            }
            temporary
                .persist_noclobber(&output_path)
                .with_context(|| format!("create WAV output {}", output_path.display()))?;
        }
    }
    Ok(())
}
