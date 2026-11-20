#!/usr/bin/env python3
"""Best-effort extraction of video and image ranges from RPF archives by signature scan.

Scans archives for media signatures and writes the surrounding ranges into the
RPF_извлечённое extraction tree next to the archives.
Files with the same name and identical content as existing outputs are skipped.
"""

from pathlib import Path
import os

HERE = Path(__file__).resolve().parent

PNG = b'\x89PNG\r\n\x1a\n'
JPEG = b'\xff\xd8\xff'
GIF87 = b'GIF87a'
GIF89 = b'GIF89a'
DDS = b'DDS '
BINK = b'BINK'
KB2N = b'KB2n'

SIG_EXT = {
    KB2N: 'bk2',
    BINK: 'bik',
    PNG: 'png',
    JPEG: 'jpg',
    GIF87: 'gif',
    GIF89: 'gif',
    DDS: 'dds',
}

VIDEO_SIGS = [KB2N, BINK]
IMAGE_SIGS = [PNG, JPEG, GIF87, GIF89, DDS]
MAX_WINDOW_VIDEO = 16 << 20
MAX_WINDOW_IMAGE = 8 << 20


def scan(path, sigs, chunk=4 << 20):
    """Sorted list of (offset, signature) hits for every occurrence of sigs in path."""
    hits = set()
    mx = max(len(s) for s in sigs)
    size = os.path.getsize(path)
    with open(path, 'rb') as f:
        pos = 0
        while pos < size:
            f.seek(pos)
            data = f.read(chunk + mx - 1)
            if not data:
                break
            for s in sigs:
                start = 0
                while True:
                    i = data.find(s, start)
                    if i < 0:
                        break
                    hits.add((pos + i, s))
                    start = i + 1
            pos += chunk
    return sorted(hits)


def identical_existing(archive, off, out):
    """True when out already contains the same bytes as the window at off."""
    try:
        with open(out, 'rb') as f:
            existing = f.read()
    except OSError:
        return False
    if not existing:
        return False
    with open(archive, 'rb') as f:
        f.seek(off)
        window = f.read(len(existing))
    return window == existing


def run(archive, sigs, outdir, max_window, refresh=False):
    archive = str(HERE.parent / archive)
    outdir.mkdir(parents=True, exist_ok=True)
    new_files = []
    duplicates = []
    skipped = []
    per_sig = {}
    for off, sig in scan(archive, sigs):
        per_sig[sig] = per_sig.get(sig, 0) + 1
        name = f'{off:08X}.{SIG_EXT[sig]}'
        out = outdir / name
        if out.exists():
            if identical_existing(archive, off, out):
                duplicates.append((name, out.stat().st_size))
                continue
            if not refresh:
                skipped.append((name, out.stat().st_size))
                continue
            out.unlink()
        with open(archive, 'rb') as f:
            f.seek(off)
            window = f.read(max_window)
        with open(out, 'wb') as g:
            g.write(window)
        new_files.append((name, len(window)))
    return new_files, duplicates, skipped, per_sig


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--refresh', action='store_true',
                        help='re-extract files that exist but differ from the archive')
    args = parser.parse_args()
    movies_dir = next(HERE.glob('*/movies'))
    jobs = [
        ('movies.rpf', VIDEO_SIGS, movies_dir, MAX_WINDOW_VIDEO),
        ('textures.rpf', IMAGE_SIGS, HERE / 'textures', MAX_WINDOW_IMAGE),
        ('common.rpf', IMAGE_SIGS, HERE / 'common', MAX_WINDOW_IMAGE),
    ]
    total_new = 0
    for archive, sigs, outdir, max_window in jobs:
        new_files, duplicates, skipped, per_sig = run(archive, sigs, outdir, max_window, refresh=args.refresh)
        print(f'== {archive} -> {outdir}')
        print(f'   new: {len(new_files)} files, {sum(s for _, s in new_files)} bytes')
        for name, size in new_files[:20]:
            print(f'   + {name} ({size} bytes)')
        if len(new_files) > 20:
            print(f'   + ... {len(new_files) - 20} more')
        print(f'   duplicates (already extracted): {len(duplicates)}', duplicates[:10])
        if skipped:
            print(f'   skipped, existing differs: {skipped[:10]}')
        print('   signature hits:', {k.hex(): v for k, v in per_sig.items()})
        total_new += sum(s for _, s in new_files)
    print(f'TOTAL new bytes: {total_new}')


if __name__ == '__main__':
    main()
