#!/usr/bin/env python3
"""Read-only MultiArc bridge for recovered RPF9 streams."""

import argparse
import csv
import fnmatch
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
import tempfile
import zlib

sys.dont_write_bytecode = True
from rpf9_archive_io import RAW_EXTENSIONS, copy_stream

ROOT = Path(__file__).resolve().parent.parent


def executable():
    for profile in ("release", "debug"):
        binary = ROOT / "rpf9" / "target" / profile / "rpf9-extract"
        if binary.is_file() and os.access(binary, os.X_OK):
            return str(binary)
    binary = shutil.which("rpf9-extract")
    if binary:
        return binary
    raise RuntimeError("rpf9-extract was not found. Build it or add it to PATH.")


def archive_identity(path):
    stat = path.stat()
    return [str(path), stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns]


def load_entries(path):
    identity = archive_identity(path)
    cache_dir = Path(tempfile.gettempdir()) / f"rpf9-doublecmd-{os.getuid()}"
    cache_dir.mkdir(mode=0o700, exist_ok=True)
    key = hashlib.sha256(json.dumps(identity, ensure_ascii=True).encode()).hexdigest()
    cache_path = cache_dir / (key + ".tsv")

    try:
        raw = cache_path.read_text(encoding="utf-8")
    except FileNotFoundError:
        proc = subprocess.run(
            [executable(), "--index-stdout", str(path)],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
        )
        if proc.returncode:
            raise RuntimeError(proc.stderr.decode("utf-8", errors="replace").strip()
                               or f"rpf9-extract exited with status {proc.returncode}")
        if archive_identity(path) != identity:
            raise RuntimeError("The archive changed while it was being scanned.")
        raw = proc.stdout.decode("utf-8")
        # Write only complete scans to the shared cache.
        temp_name = None
        try:
            with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=cache_dir,
                                             delete=False) as temp:
                temp.write(raw)
                temp_name = temp.name
            os.replace(temp_name, cache_path)
        finally:
            if temp_name and os.path.exists(temp_name):
                os.unlink(temp_name)

    reader = csv.DictReader(io.StringIO(raw), delimiter="\t")
    if reader.fieldnames != ["archive", "offset", "length", "size", "parts", "ext", "kind", "note"]:
        raise RuntimeError("Unexpected rpf9-extract index format.")
    entries = []
    for row in reader:
        owner = int(row["archive"], 0)
        offset = int(row["offset"], 0)
        stored = int(row["length"])
        decoded = int(row["size"])
        ext = row["ext"]
        if not ext.isascii() or not ext.isalnum():
            raise RuntimeError("Invalid entry extension in index.")
        if owner < 0 or offset < 0 or stored <= 0 or decoded < 0 or offset + stored > identity[3]:
            raise RuntimeError("Invalid entry bounds in index.")
        name = f"h{owner:06x}/{offset:08x}.{ext}"
        entries.append((name, offset, stored, decoded, ext))
    return entries


def selected_entries(entries, selection):
    requested = []
    with open(selection, encoding="utf-8-sig") as selected:
        for line in selected:
            name = line.rstrip("\r\n")
            if len(name) >= 2 and name[0] == name[-1] == '"':
                name = name[1:-1]
            name = name.replace("\\", "/").lstrip("/")
            if name:
                requested.append(name)
    if not requested:
        raise RuntimeError("No archive entries selected.")
    matches = []
    found = [False] * len(requested)
    for entry in entries:
        name = entry[0]
        matched = False
        for i, wanted in enumerate(requested):
            if (name == wanted or name.startswith(wanted.rstrip("/") + "/")
                    or fnmatch.fnmatchcase(name, wanted)):
                found[i] = True
                matched = True
        if matched:
            matches.append(entry)
    if not all(found):
        missing = next(wanted for i, wanted in enumerate(requested) if not found[i])
        raise RuntimeError(f"Archive entry not found: {missing}")
    return matches


def copy_entry(archive, entry, flat):
    name, offset, stored, decoded, ext = entry
    relative = PurePosixPath(name)
    target = Path(relative.name) if flat else Path(*relative.parts)
    target.parent.mkdir(parents=True, exist_ok=True)
    # Generated names have two components, so no archive name can escape cwd.
    with target.open("xb") as output:
        try:
            copy_stream(archive, output, offset, stored, decoded, ext)
        except BaseException:
            output.close()
            target.unlink(missing_ok=True)
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("list", "extract", "extract-flat"))
    parser.add_argument("archive", type=Path)
    parser.add_argument("selection", nargs="?", type=Path)
    args = parser.parse_args()
    try:
        archive = args.archive.resolve(strict=True)
        entries = load_entries(archive)
        if args.operation == "list":
            if args.selection:
                parser.error("list does not accept a selection file")
            print("RPF9-BEGIN")
            for name, _, stored, decoded, ext in entries:
                size = stored if ext in RAW_EXTENSIONS else decoded
                print(f"{size:12d} {stored:12d} {name}")
            print("RPF9-END")
        else:
            if not args.selection:
                parser.error("extract requires a selection file")
            for entry in selected_entries(entries, args.selection):
                copy_entry(archive, entry, args.operation == "extract-flat")
        return 0
    except (OSError, ValueError, RuntimeError, zlib.error) as error:
        print(f"rpf9-doublecmd-archive: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
