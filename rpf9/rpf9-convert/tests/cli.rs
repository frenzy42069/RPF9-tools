use std::{fs, path::Path, process::Command};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;

fn converter() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rpf9-convert"))
}

#[test]
fn model_encodes_the_same_geometry_in_glb_and_gltf() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("triangle mesh.obj");
    fs::write(&input, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();

    assert!(converter()
        .arg("model")
        .arg(&input)
        .status()
        .unwrap()
        .success());
    assert!(converter()
        .arg("model")
        .arg(&input)
        .args(["--format", "gltf"])
        .status()
        .unwrap()
        .success());

    let glb = fs::read(input.with_extension("glb")).unwrap();
    let word = |offset| u32::from_le_bytes(glb[offset..offset + 4].try_into().unwrap()) as usize;
    assert_eq!(&glb[..4], b"glTF");
    assert_eq!(word(4), 2);
    assert_eq!(word(8), glb.len());
    assert_eq!(&glb[16..20], b"JSON");
    let bin = 20 + word(12);
    assert_eq!(&glb[bin + 4..bin + 8], b"BIN\0");
    assert_eq!(word(bin), 48);

    let glb_json: Value = serde_json::from_slice(&glb[20..bin]).unwrap();
    let mut gltf_json: Value =
        serde_json::from_slice(&fs::read(input.with_extension("gltf")).unwrap()).unwrap();
    let uri = gltf_json["buffers"][0]
        .as_object_mut()
        .unwrap()
        .remove("uri")
        .unwrap();
    assert_eq!(glb_json, gltf_json);
    let (_, data) = uri.as_str().unwrap().split_once(',').unwrap();
    assert_eq!(STANDARD.decode(data).unwrap(), &glb[bin + 8..]);
    assert_eq!(&glb[bin + 44..], &[0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0]);
}

#[test]
fn texture_defaults_to_png_and_preserves_an_existing_output() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("texture.bmp");
    let output_path = input.with_extension("png");
    image::RgbaImage::from_pixel(2, 1, image::Rgba([8, 16, 32, 255]))
        .save(&input)
        .unwrap();

    let convert = |input: &Path| converter().arg("texture").arg(input).output().unwrap();
    let result = convert(&input);
    assert!(result.status.success(), "{:?}", result.stderr);
    assert!(result.stdout.is_empty());
    assert_eq!(
        image::open(&output_path).unwrap().into_rgba8().into_raw(),
        [8, 16, 32, 255, 8, 16, 32, 255]
    );
    let existing = fs::read(&output_path).unwrap();

    let result = convert(&input);
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("create output"));
    assert_eq!(fs::read(output_path).unwrap(), existing);
}

#[test]
fn exports_drawable_meshlets_from_the_local_levels_archive() {
    let archive = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game/levels.rpf");
    if !archive.is_file() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("drawable.obj");
    let result = converter()
        .arg("model")
        .arg(archive)
        .args(["--offset", "0xf9c02010", "--format", "obj", "--output"])
        .arg(&destination)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let model = fs::read_to_string(destination).unwrap();
    assert_eq!(
        model.lines().filter(|line| line.starts_with("v ")).count(),
        330
    );
    assert_eq!(
        model.lines().filter(|line| line.starts_with("f ")).count(),
        350
    );
}
