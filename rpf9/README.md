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
  writes into `out/h<archive:06x>/`, with `index.tsv` and `report.txt` at the
  output root. Supported drawables use their embedded names. Unnamed entries
  use offsets, and duplicate names get an offset suffix. Use `--offset-names`
  for the previous offset-based filenames throughout.
- `rpf9-fuse` — FUSE mount: `rpf9-fuse [OPTIONS] <archive-or-build-directory> <mntpoint>`.
  The mount uses the same filenames as extraction under
  `<file>/h<archive:06x>/` for a single archive. Build-directory mounts
  show each RPF as a folder and load large archives one range at a time.
- `rpf9-convert` converts supported Xbox drawables and texture entries, and
  accepts OBJ geometry and common image files.
  See [converter examples](#model-and-texture-conversion).
- `rpf9-navmesh` — Navmesh → Wavefront OBJ:
  `rpf9-navmesh convert <input> [-o output.obj] [--offset N] [-v]`
  Discovers navmesh geometry in an extracted navmesh resource
  (e.g. `results/navmeshes/h*/…/*.rsc`): the longest run of plausible f32
  triples with the most substantial world-like coordinates becomes the
  vertices, and the longest run of nonzero index values (u16/u32) becomes
  the faces. Heuristic — see `rpf9-navmesh/src/mesh.rs`; nothing is decoded
  from the native format yet.
- `rpf9-shader` — Shader data decompiler:
  `rpf9-shader <input> [-o report.txt] [--offset N]`
  Decompiles shader data — the `PS\0\x09` PSO caches from `shaders.rpf`
  (cache id, version, hash, header words) and `SGD5` blobs (hash, count,
  header words, entry table). Program-table semantics are not decoded.
  Fixtures: `rpf9-shader results/shaders/h000000/10579000.ps09` or
  `rpf9-shader results/shaders/sgd5_a.bin`.

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

## Unified CLI

Build the workspace or install the `rpf9` package. The catalog prints TSV by
default and JSON with `--json`. Use the same selection flags for batch exports.
Outputs include archive offsets or texture indices where needed to distinguish
assets with the same name. The scan defaults to 256 MiB and 100 results.
`--all` scans to EOF. Use `--limit` to cap results and `--name` to filter names.

```sh
cargo build --manifest-path rpf9/Cargo.toml --release --workspace
rpf9/target/release/rpf9 list game/levels.rpf --kind model --all --limit 10 --json
rpf9/target/release/rpf9 export game/levels.rpf --kind model --all --name laundry -o models
rpf9/target/release/rpf9 export game/textures.rpf --kind texture --name pistol_glass -o textures
rpf9/target/release/rpf9 export game/data.rpf --kind localization --offset 0x1219010 -o strings
rpf9/target/release/rpf9 export game/movies.rpf --kind video --offset 0x7000 -f bk2 -o videos
rpf9/target/release/rpf9 extract --index-stdout game/script.rpf
rpf9/target/release/rpf9 extract game/levels.rpf extracted-levels
```

Models default to GLB, textures to PNG, localization to JSONL, and video to
MP4. Supported formats appear under `rpf9 export --help`. Use
`--ffmpeg=/path/to/ffmpeg` to choose an FFmpeg build. For Xbox Bink2 to MP4,
use the [Bink2 FFmpeg companion](#bink2-ffmpeg-companion).
`--format bk2` copies the original clip. `rpf9 mount` delegates to the
workspace FUSE tool.

Audio defaults to original packets. On Linux, build the ADAT helper beside
the release CLI or specify it with `--audio-extractor`. With libopus
available, the helper wraps Opus packets without re-encoding and retains
XMA2 in `.xma`. `--format wav` also writes decoded WAV files using FFmpeg.

```sh
cc -O2 -std=c11 scripts/rpf9_extract_audio.c -ldl -o rpf9/target/release/rpf9-extract-audio
rpf9/target/release/rpf9 export game/audio.rpf --kind audio --offset 0x13da000 -o audio
```

Native drawables can include UVs. Material links and skinning still require
format coverage. Texture export covers supported layouts, including named
BC2, BC3, BC5, and BC7 textures from embedded dictionaries and TTPF packs.
Localization rows carry recovered keys and use a null locale when the
resource does not identify it.

### Bink2 FFmpeg companion

Build a standalone FFmpeg on x86-64 Linux with Bash, Git, GCC, GNU Make, and
`pkg-config`. The Windows x64 build also needs the `x86_64-w64-mingw32-`
GCC and binutils toolchain. Install NASM to enable x86 assembly optimizations.
Run from the repository root.

```sh
scripts/bink2-ffmpeg/build.sh linux-x86_64 rpf9/target/bink2-ffmpeg
scripts/bink2-ffmpeg/build.sh windows-x86_64 rpf9/target/bink2-ffmpeg
```

Use `ffmpeg` or `ffmpeg.exe` from the matching build directory with the export
command. Linux FUSE uses the same executable for on-demand previews.

```sh
ffmpeg="$PWD/rpf9/target/bink2-ffmpeg/rpf9-bink2-ffmpeg-linux-x86_64/ffmpeg"
rpf9 export movies.rpf --kind video --offset 0x7000 --ffmpeg="$ffmpeg" -o previews
rpf9 mount /path/to/build /path/to/mountpoint --readable --ffmpeg "$ffmpeg"
```

The companion converts Bink video and audio to H.264/AAC MP4. For other output
formats or asset conversions, pass a build with the required codecs and muxers.
Some Xbox clips still fail to decode or have audio dropouts. Use `--format bk2`
to export the original clip in those cases.
The [mobipeg](https://github.com/quatric/mobipeg) Bink 2 decoder is
AGPL-3.0-or-later. Distribute the companion binary archive alongside its
matching `-source.tar.gz`, which contains both pinned source trees and the
build script. Unpack the source archive into a new directory to rebuild offline.

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
vertex maps. The exporter writes triangle geometry and native UVs where
available. Materials, skinning, and drawable normals are currently unsupported.

Texture output defaults to PNG. Other choices are `jpeg` (alias `jpg`),
`webp`, `bmp`, and `tga`. The reader accepts common images and supported
RDS5/TTPF entries. Use `--entry N` to select a zero-based texture index
(default 0). Xbox texture support covers BC2, BC3, BC5, and BC7 with raw or zlib chunks and
4 KiB or 64 KiB tiling, with both dimensions divisible by 256. The exporter
writes one image per conversion.

Textures use the native encoder by default. Use `--ffmpeg` to look up
`ffmpeg` through PATH, or `--ffmpeg=/path/to/ffmpeg` to select an executable.
FFmpeg encodes the decoded texture and needs the encoder for the chosen
format. The converter writes beside the input by default and uses any
explicitly selected texture index or drawable offset in the output filename.
It refuses to overwrite existing files.

### Linux file manager previews

Mount the entire build once to browse it in Double Commander or any other
file manager. The mountpoint should be an empty directory and the Linux host
needs `/dev/fuse`.

```sh
cargo build --manifest-path rpf9/Cargo.toml --release -p rpf9 -p rpf9-fuse
cc -O2 -std=c11 scripts/rpf9_extract_audio.c -ldl -o rpf9/target/release/rpf9-extract-audio
rpf9/target/release/rpf9 mount /path/to/build /path/to/mountpoint --readable
```

The directory layout follows the build directory. Each `.rpf` is a folder.
Large archives open at a list of 64 MiB ranges. The first visit to a range
scans it and saves the listing for later visits and mounts. In `--readable`
mode, supported `.ttpf` packs and embedded texture dictionaries open as
folders of PNGs, ADAT banks have folders of Opus or XMA tracks, and named
drawables expose GLB previews. The raw entries remain available. Pass
`--ffmpeg /path/to/bink2-capable/ffmpeg` to add silent MP4 previews. Xbox Bink2
audio can decode as loud noise with the current companion, so mounted previews
omit the audio track.

The first open of a converted file prepares a seekable cached copy. MP4
previews show the source clip size until the converted size is available.
`--cache-dir PATH` changes the cache location from
`${XDG_CACHE_HOME:-$HOME/.cache}/rpf9`. `--cache-bytes N` sets the media
budget, which defaults to 10 GiB. Folder indexes use a separate namespace
and survive media eviction. Keep input archives stable while mounted.

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
