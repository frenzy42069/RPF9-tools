#!/usr/bin/env python3
"""Preview RPF9 models or TTPF textures from a Linux file manager."""

import argparse
import csv
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
SCAN_BYTES = 64 * 1024 * 1024
PAGE_SIZE = 100
IMPORT_GLB = """
import sys
import bpy

path = sys.argv[sys.argv.index('--') + 1]
bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete(use_global=False)
if 'FINISHED' not in bpy.ops.import_scene.gltf(filepath=path):
    raise RuntimeError('Blender could not import the preview GLB')
if not [obj for obj in bpy.context.scene.objects if obj.type == 'MESH']:
    raise RuntimeError('The imported preview has no meshes')
if not bpy.app.background and bpy.context.screen:
    for area in bpy.context.screen.areas:
        if area.type != 'VIEW_3D':
            continue
        region = next((region for region in area.regions if region.type == 'WINDOW'), None)
        if region:
            with bpy.context.temp_override(area=area, region=region):
                bpy.ops.view3d.view_all(center=True)
            break
"""


def converter():
    for profile in ("release", "debug"):
        binary = ROOT / "rpf9" / "target" / profile / "rpf9-convert"
        if binary.is_file():
            return str(binary)
    binary = shutil.which("rpf9-convert")
    if binary:
        return binary
    raise RuntimeError("Build rpf9-convert or put it on PATH.")


def run(*argv):
    result = subprocess.run(argv, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or f"{argv[0]} exited with {result.returncode}")
    return result.stdout


def sample_start(archive):
    report = ROOT / "game/RPF_извлечённое/rpf9_object_starts.tsv"
    if archive != ROOT / "game/levels.rpf" or not report.is_file():
        return 0
    with report.open(encoding="utf-8", newline="") as source:
        rows = csv.reader(source, delimiter="\t")
        next(rows, None)
        row = next((row for row in rows if row and row[0] == archive.name), None)
    return int(row[1], 16) if row else 0


def choose_model(binary, archive):
    fuzzel = shutil.which("fuzzel")
    if not fuzzel:
        raise RuntimeError("The searchable model picker requires fuzzel.")
    size = archive.stat().st_size
    start = sample_start(archive)
    while start < size:
        models = json.loads(run(
            binary, "list-models", archive, "--start", str(start),
            "--scan-bytes", str(SCAN_BYTES), "--limit", str(PAGE_SIZE),
        ))
        # The offset stays in the CLI. The picker only shows names and triangle counts.
        rows = [
            f"{' '.join(model['name'].split())}    ({model['triangles']:,} triangles)"
            for model in models
        ]
        if len(models) == PAGE_SIZE:
            next_start = models[-1]["offset"] + 16
        else:
            next_start = start + SCAN_BYTES
        if next_start < size:
            rows.append("Scan next archive section")
        if not rows:
            break
        picker = subprocess.run(
            [fuzzel, "--dmenu", "--index", "--only-match", "--log-level=error", "--prompt", "Model: ",
             "--mesg", f"{archive.name} · {len(models)} models in this section"],
            input="\n".join(rows) + "\n", capture_output=True, text=True,
        )
        if picker.returncode == 1 and not picker.stderr.strip():
            return None
        if picker.returncode:
            raise RuntimeError(picker.stderr.strip() or "The model picker could not open.")
        choice = int(picker.stdout)
        if 0 <= choice < len(models):
            return models[choice]["offset"]
        if choice != len(models) or next_start >= size:
            raise RuntimeError("The model picker returned an invalid selection.")
        start = next_start
    raise RuntimeError("No supported drawables found in the remaining archive sections.")


def open_model(path):
    run("blender", "--factory-startup", "--python-exit-code", "1",
        "--python-expr", IMPORT_GLB, "--", path)


def preview(path):
    extension = path.suffix.lower()
    if extension in (".glb", ".gltf"):
        open_model(path)
        return

    binary = converter()
    with tempfile.TemporaryDirectory(prefix="rpf9-preview-") as directory:
        if extension in (".rpf", ".obj"):
            output = Path(directory) / "preview.glb"
            options = []
            if extension == ".rpf":
                offset = choose_model(binary, path)
                if offset is None:
                    return
                options = ["--offset", str(offset)]
            run(binary, "model", path, *options, "--output", output)
            open_model(output)
        elif extension == ".ttpf":
            output = Path(directory) / "preview.png"
            run(binary, "texture", path, "--output", output)
            run("mpv", "--force-window=yes", "--image-display-duration=inf",
                f"--title={path.name}", "--", output)
        else:
            raise RuntimeError(f"Unsupported preview file: {path.name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("file", type=Path)
    args = parser.parse_args()
    try:
        preview(args.file.resolve(strict=True))
        return 0
    except (OSError, ValueError, RuntimeError) as error:
        message = str(error)
        print(message, file=sys.stderr)
        if zenity := shutil.which("zenity"):
            subprocess.run([zenity, "--error", "--no-markup", "--title=RPF9 Preview",
                            f"--text={message[-2000:]}"], check=False)
        return 1


if __name__ == "__main__":
    sys.exit(main())
