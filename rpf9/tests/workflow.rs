use std::{fs, path::PathBuf, process::Command};

use serde_json::Value;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rpf9"))
}

#[test]
fn catalog_and_batch_copy_preserve_bink_ranges() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("archive with spaces.rpf");
    let mut data = vec![0; 80];
    data[..4].copy_from_slice(b"9FPR");
    for offset in [16, 48] {
        data[offset..offset + 4].copy_from_slice(b"KB2n");
        data[offset + 4..offset + 8].copy_from_slice(&16_u32.to_le_bytes());
        data[offset + 8..offset + 12].copy_from_slice(&1_u32.to_le_bytes());
        data[offset + 16] = offset as u8;
    }
    fs::write(&input, &data).unwrap();

    let result = cli()
        .arg("list")
        .arg(&input)
        .args(["--kind", "video", "--json"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let entries: Vec<Value> = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["offset"], 16);
    assert_eq!(entries[1]["offset"], 48);

    let target = temp.path().join("clips");
    let run = || {
        cli()
            .arg("export")
            .arg(&input)
            .args(["--kind", "video", "--format", "bk2", "--output-dir"])
            .arg(&target)
            .output()
            .unwrap()
    };
    let result = run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut outputs = fs::read_dir(&target)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    outputs.sort();
    assert_eq!(outputs.len(), 2);
    for (path, offset) in outputs.iter().zip([16, 48]) {
        assert_eq!(fs::read(path).unwrap(), data[offset..offset + 24]);
    }
    assert!(!run().status.success());
    assert_eq!(fs::read_dir(target).unwrap().count(), 2);
}

#[test]
fn embedded_textures_are_searchable_and_exportable_by_name() {
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../game/textures.rpf");
    if !input.is_file() {
        return;
    }
    let target = tempfile::tempdir().unwrap();
    let result = cli()
        .arg("export")
        .arg(input)
        .args([
            "--kind",
            "texture",
            "--scan-bytes",
            "0x40000",
            "--name",
            "pistol_glass_crack_decals",
            "--limit",
            "1",
            "--output-dir",
        ])
        .arg(target.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let entries = fs::read_dir(target.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
    assert!(entries[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains("pistol_glass_crack_decals"));
    assert_eq!(&fs::read(&entries[0]).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
}
