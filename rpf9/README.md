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
