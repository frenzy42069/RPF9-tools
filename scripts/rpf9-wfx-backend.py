#!/usr/bin/env python3
"""Browse and extract one range at a time through Double Commander's RPF9 WFX."""

import argparse
import csv
import hashlib
import io
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
import zlib

sys.dont_write_bytecode = True
from rpf9_archive_io import RAW_EXTENSIONS, copy_stream

ROOT = Path(__file__).resolve().parent.parent
PAGE_BYTES = 64 * 1024 * 1024
BLOCK_BYTES = 16 * PAGE_BYTES
INDEX_HEADER = ["archive", "offset", "length", "size", "parts", "ext", "kind", "note"]
SLOT = re.compile(r"[a-zA-Z0-9_.-]+--[0-9a-f]{16}")
PAGE = re.compile(r"range-([0-9a-f]{10,16})")
BLOCK = re.compile(r"block-([0-9a-f]{10,16})")


def state_dir():
    directory = Path(tempfile.gettempdir()) / f"rpf9-wfx-{os.getuid()}"
    directory.mkdir(mode=0o700, exist_ok=True)
    info = directory.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise RuntimeError(f"Cannot use private archive state directory {directory}")
    return directory


def write_atomic(destination, write, overwrite):
    with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as temp:
        pending = Path(temp.name)
        try:
            write(temp)
            temp.flush()
            if overwrite:
                os.replace(pending, destination)
            else:
                os.link(pending, destination)
        finally:
            pending.unlink(missing_ok=True)


def identity(path):
    info = path.stat()
    if not stat.S_ISREG(info.st_mode):
        raise RuntimeError("Select a regular RPF archive")
    return [str(path), info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns]


def register(directory, path):
    path = path.resolve(strict=True)
    info = identity(path)
    with path.open("rb") as archive:
        if archive.read(4) != b"9FPR":
            raise RuntimeError("Expected an RPF9 archive with a 9FPR header")
    tag = hashlib.sha256(json.dumps(info).encode()).hexdigest()[:16]
    name = re.sub(r"[^a-zA-Z0-9_.-]", "_", path.name)[:90]
    slot = f"{name}--{tag}"
    destination = directory / f"{slot}.json"
    try:
        write_atomic(destination, lambda file: file.write(json.dumps(info).encode()), False)
    except FileExistsError:
        pass
    return slot


def archive_info(directory, slot):
    if not SLOT.fullmatch(slot):
        raise RuntimeError("Invalid archive folder name")
    info = json.loads((directory / f"{slot}.json").read_bytes())
    path = Path(info[0])
    if identity(path) != info:
        raise RuntimeError("The archive changed. Reopen it from Double Commander.")
    return path, info[3]


def split_remote(remote):
    return [part for part in remote.replace("\\", "/").split("/") if part]


def span(path_part, pattern, alignment, low, high):
    match = pattern.fullmatch(path_part)
    if not match:
        raise RuntimeError("Invalid archive range")
    start = int(match[1], 16)
    if start < low or start >= high or start % alignment:
        raise RuntimeError("Archive range is out of bounds")
    return start


def resolve_folder(parts, size):
    if size <= PAGE_BYTES:
        if parts:
            raise RuntimeError("Unexpected subfolder in small archive")
        return 0, size
    if size > BLOCK_BYTES:
        if not parts:
            return "block", 0, size
        block = span(parts[0], BLOCK, BLOCK_BYTES, 0, size)
        high = min(block + BLOCK_BYTES, size)
        if len(parts) == 1:
            return "range", block, high
        if len(parts) != 2:
            raise RuntimeError("Unexpected subfolder")
        start = span(parts[1], PAGE, PAGE_BYTES, block, high)
        return start, min(size - start, PAGE_BYTES)
    if not parts:
        return "range", 0, size
    if len(parts) != 1:
        raise RuntimeError("Unexpected subfolder")
    start = span(parts[0], PAGE, PAGE_BYTES, 0, size)
    return start, min(size - start, PAGE_BYTES)


def index_page(directory, slot, path, start, length):
    cache = directory / f"{slot}.{start:010x}.tsv"
    try:
        data = cache.read_text()
    except FileNotFoundError:
        binary = ROOT / "rpf9/target/release/rpf9-extract"
        output = subprocess.run(
            [binary, "--range-stdout", str(start), str(length), path],
            capture_output=True, check=False,
        )
        if output.returncode:
            raise RuntimeError(output.stderr.decode(errors="replace").strip() or "RPF9 scan failed")
        archive_info(directory, slot)
        data = output.stdout.decode()
        try:
            write_atomic(cache, lambda file: file.write(output.stdout), False)
        except FileExistsError:
            pass
    reader = csv.DictReader(io.StringIO(data), delimiter="\t")
    if reader.fieldnames != INDEX_HEADER:
        raise RuntimeError("Unexpected RPF9 index format")
    entries = {}
    for row in reader:
        ext = row["ext"]
        offset, stored, decoded = (int(row["offset"], 0), int(row["length"]), int(row["size"]))
        if not ext or not ext.isascii() or not ext.isalnum():
            raise RuntimeError("Invalid entry extension")
        if not start <= offset < start + length or stored <= 0 or decoded < 0:
            raise RuntimeError("Index entry is out of bounds")
        entries.setdefault(f"{offset:08x}.{ext}", (offset, stored, decoded, ext))
    return entries


def list_folder(directory, remote):
    parts = split_remote(remote)
    if not parts:
        for archive in sorted((ROOT / "game").glob("*.rpf")):
            try:
                register(directory, archive)
            except (OSError, RuntimeError):
                continue
        for registration in sorted(directory.glob("*.json")):
            if SLOT.fullmatch(registration.stem):
                print(f"D\t0\t{registration.stem}")
        return
    slot, *folder = parts
    path, size = archive_info(directory, slot)
    selection = resolve_folder(folder, size)
    if len(selection) == 3:
        kind, start, end = selection
        step = BLOCK_BYTES if kind == "block" else PAGE_BYTES
        for offset in range(start, end, step):
            print(f"D\t0\t{kind}-{offset:010x}")
        return
    for name, (_, stored, decoded, ext) in index_page(directory, slot, path, *selection).items():
        print(f"F\t{stored if ext in RAW_EXTENSIONS else decoded}\t{name}")


def extract(directory, remote, local, overwrite):
    parts = split_remote(remote)
    if len(parts) < 2:
        raise RuntimeError("Select an archive file")
    slot, *folder, name = parts
    path, size = archive_info(directory, slot)
    selection = resolve_folder(folder, size)
    if len(selection) != 2:
        raise RuntimeError("Select a file within a range")
    entries = index_page(directory, slot, path, *selection)
    if name not in entries:
        raise RuntimeError("Archive entry was not found")
    offset, stored, decoded, ext = entries[name]
    if offset + stored > size:
        raise RuntimeError("Archive entry extends beyond the file")
    def write(file):
        copy_stream(path, file, offset, stored, decoded, ext)
        archive_info(directory, slot)
    write_atomic(local, write, overwrite)


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    commands = cli.add_subparsers(dest="command", required=True)
    commands.add_parser("register").add_argument("archive", type=Path)
    commands.add_parser("list").add_argument("remote")
    unpack = commands.add_parser("extract")
    unpack.add_argument("remote")
    unpack.add_argument("local", type=Path)
    unpack.add_argument("--overwrite", action="store_true")
    args = cli.parse_args()
    try:
        directory = state_dir()
        if args.command == "register":
            print(register(directory, args.archive))
        elif args.command == "list":
            list_folder(directory, args.remote)
        else:
            extract(directory, args.remote, args.local, args.overwrite)
        return 0
    except (OSError, RuntimeError, ValueError, subprocess.CalledProcessError, zlib.error) as error:
        print(f"rpf9-wfx: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
