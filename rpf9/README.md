# rpf9 — RPF9 tools (decryption-less dearchivator)

Rust workspace for recovering resources from RPF9 (`9FPR`) archives without
the RPF9 native directory. The native index (original names, directory
links, entry order, physical offset/size encoding) is encrypted — see
`format_desc.md` ("Current results", "Native index") and
`game/RPF9_FORMAT.en.md`.

## Tools

- `rpf9-core` — scanner/recovery library:
  - header parsing/validation (`Header::parse`, offs/rpf9-extract style),
  - archive extents (`Archive`),
  - zlib stream walk + Adler-32 verification (`stream::walk`),
  - media carve (movies `KB2`/`BINK`, ADAT banks, PNG, RIFF) from gaps,
  - classification by decompressed signature (`kinds::classify`),
  - offs-compatible outputs (`index.tsv`, `report.txt`).
- `rpf9-extract` — static dearchivator:
  `rpf9-extract [-n|--list] [-v|--verbose] <rpf> [outdir]`
  writes `out/h<archive:06x>/<offset:08x>.<ext>` plus `index.tsv` and
  `report.txt`. Offset-named: original names are unavailable until the
  native directory is decoded.
- `rpf9-fuse` — FUSE mount: `rpf9-fuse <rpf> [mntpoint]`.
  Mount point tree: `<file>/h<archive:06x>/<offset:08x>.<ext>` — open the
  archive as a folder; resources decode on read.
- `rpf9-convert` converts supported Xbox drawables and texture entries, and
  accepts OBJ geometry and common image files.
  See [converter examples](#model-and-texture-conversion).

Requires: `cargo` to build; `fuser` tier also needs `fusermount3` +
`/dev/fuse`.

## Build & test

    cargo build            # debug binaries in target/debug
    cargo build --release
    cargo test             # rpf9-core tests vs local game samples (game/ folder)

## Examples

    ./target/debug/rpf9-extract game/data.rpf /tmp/out_data
    ./target/debug/rpf9-extract -n game/data.rpf
    mkdir /tmp/fuse_data && ./target/debug/rpf9-fuse game/data.rpf /tmp/fuse_data
    ls /tmp/fuse_data/data.rpf/h11de000 | head
    fusermount3 -u /tmp/fuse_data

## Model and texture conversion

Build from the workspace directory with `cargo build -p rpf9-convert`, or run
these examples from the repository root.

```sh
cargo run --manifest-path rpf9/Cargo.toml -p rpf9-convert -- model mesh.obj
cargo run --manifest-path rpf9/Cargo.toml -p rpf9-convert -- model mesh.obj --format gltf -o mesh.gltf
cargo run --manifest-path rpf9/Cargo.toml -p rpf9-convert -- model game/levels.rpf --offset 0xf9c02010 -o drawable.glb
cargo run --manifest-path rpf9/Cargo.toml -p rpf9-convert -- texture game/data/ttdb/ui/platform.ttpf -o controller.png
cargo run --manifest-path rpf9/Cargo.toml -p rpf9-convert -- texture game/pack/core_restofworld001/data/ttdb/ui/ttdb_ui.ttpf --entry 1 --format webp --ffmpeg -o icon.webp
```

Model output defaults to GLB (glTF 2.0). Select `gltf` for JSON with an
embedded buffer or `obj` for Wavefront OBJ. OBJ input preserves positions,
normals, and UVs. For RAGE drawables, pass the offset of the zlib stream using
`--offset` in decimal or hexadecimal. The decoder supports the observed
`.#dr` layout with two zlib streams, packed position data, and 16-bit meshlet
vertex maps. Drawable export writes triangle geometry. Materials, skinning,
and drawable normals and UVs are currently unsupported.

Texture output defaults to PNG. Other choices are `jpeg` (alias `jpg`),
`webp`, `bmp`, and `tga`. The reader accepts common images and supported
RDS5/TTPF entries. Use `--entry N` to select a zero-based texture index
(default 0). Xbox texture support covers BC3 and BC7 with zlib chunks and
4 KiB or 64 KiB tiling, with both dimensions divisible by 256. The exporter
writes one image per conversion.

Textures use the native encoder by default. Use `--ffmpeg` to look up
`ffmpeg` through PATH, or `--ffmpeg=/path/to/ffmpeg` to select an executable.
FFmpeg encodes the decoded texture and needs the encoder for the chosen
format. The converter writes beside the input by default and uses any
explicitly selected texture index or drawable offset in the output filename.
It refuses to overwrite existing files.

### Linux file manager previews

The RPF9 WFX plugin opens archives as read-only folders in Double Commander.
Large archives open at a list of offset folders. Entering a 64 MiB range
scans just that range and caches its listing. Opening or copying an entry
extracts only that entry. Small archives open directly at their files.

Build the release extractor and compile the Linux plugin from the repository
root.

```sh
cargo build --manifest-path rpf9/Cargo.toml -p rpf9-extract --release
cc -std=c11 -fPIC -shared "-DRPF9_WFX_BACKEND=\"$PWD/scripts/rpf9-wfx-backend.py\"" \
  -o rpf9/target/release/rpf9.wfx scripts/rpf9-wfx.c
```

Register the plugin as `RPF9` under **Plugins → WFX plugins**. Set the `.rpf`
**Open** action to `cm_ExecuteScript` with the absolute path to
`scripts/rpf9-doublecmd-open.lua` as its parameter. Disable the RPF9 MultiArc
entry to have Double Commander use that action. You can also open **RPF9**
from the virtual filesystem list.

The MultiArc adapter is available for cases where a full index is wanted.
Use `scripts/rpf9-doublecmd-archive` as its external archiver on Linux.

```ini
Extension=rpf
ID=39 46 50 52
IDPos=0
Start=^RPF9-BEGIN
End=^RPF9-END
Format0=zzzzzzzzzzzz pppppppppppp n+
List=%P list %AQ %OU
Extract=%P extract %AQ %LQU
ExtractWithoutPath=%P extract-flat %AQ %LQU
```

The adapter lists recovered entries by archive and stream offset. The first
open scans the archive. Later opens reuse the cached listing, and viewing or
copying an entry extracts only the selected data.

For previews, build the release converter and set a custom file action to
call `scripts/rpf9-preview.py` with the selected file's full path. In Double
Commander the path parameter is `%p`.

The launcher opens a searchable model picker for `.rpf` files using fuzzel.
It scans one archive section at a time and opens the selected model in
Blender. It also opens `.glb`, `.gltf`, and `.obj` in Blender. For `.ttpf` files,
it previews entry zero in mpv when the texture uses a supported layout.
