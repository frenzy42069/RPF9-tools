#!/usr/bin/env python3
"""List and extract the checked PSIN catalog; does not read the native RPF9 TOC."""
import argparse
import csv
import json
from pathlib import Path
import struct
import zlib

HERE = Path(__file__).resolve().parent
LIMIT = 16 << 20


def catalog(source, base):
    try:
        key = source.resolve().relative_to(HERE.parent).as_posix()
    except ValueError as exc:
        raise ValueError('archive is outside the indexed bundle') from exc
    with (HERE / 'rpf9_psin_observed_entries.tsv').open(encoding='utf-8-sig', newline='') as f:
        rows = [r for r in csv.DictReader(f, delimiter='\t')
                if r['archive'] == key and int(r['base'], 16) == base]
    if not rows:
        raise ValueError('no verified PSIN catalog for this archive and base')
    scope = json.loads((HERE / 'inventory.json').read_text())[key]
    if source.stat().st_size != scope['size']:
        raise ValueError('file size differs from indexed input')
    with source.open('rb') as f:
        f.seek(base)
        head = f.read(40)
    if len(head) != 40 or head[:4] != b'9FPR' or struct.unpack_from('<I', head, 36)[0] != len(rows):
        raise ValueError('RPF header does not match catalog')
    entries = []
    for i, r in enumerate(rows):
        spans = json.loads(r['stream_ranges'])
        if int(r['physical_ordinal']) != i or len(spans) != int(r['stream_count']) or not spans:
            raise ValueError('invalid catalog row')
        entries.append({'ordinal': i, 'stored_start': r['stored_start'],
                        'stored_end_exclusive': r['stored_end_exclusive'], 'streams': spans})
    return key, scope, entries


def checksum_psin(data):
    if len(data) < 28 or data[:4] != b'PSIN' or data[-20:-16] != b'CHKS' or data[-4:] != b'zppp':
        raise ValueError('not a complete PSIN')
    size, n, expected = struct.unpack_from('>III', data, len(data)-16)
    if size != 20 or n != len(data):
        raise ValueError('PSIN size mismatch')
    h, mask = 0x3FAC7125, 0xFFFFFFFF
    for b in data[:-12] + bytes(8) + data[-4:]:
        h = (h + (b if b < 128 else b-256)) & mask
        h = (h + (h << 10)) & mask
        h ^= h >> 6
    h = (h + (h << 3)) & mask
    h ^= h >> 11
    h = (h + (h << 15)) & mask
    if h != expected:
        raise ValueError('PSIN CHKS mismatch')
    return h


def extract(source, scope, entry):
    data = bytearray()
    previous_end = 0
    with source.open('rb') as f:
        for a, b in entry['streams']:
            lo, hi = int(a, 16), int(b, 16)
            if not (previous_end <= lo < hi <= scope['size'] and hi-lo <= LIMIT):
                raise ValueError('invalid compressed bounds')
            if not any(s <= lo < hi <= e for s, e in scope['ranges']):
                raise ValueError('stream is outside the recorded verified torrent ranges')
            f.seek(lo)
            packed = f.read(hi-lo)
            if len(packed) != hi-lo or len(data) >= LIMIT:
                raise ValueError('truncated input or output limit reached')
            d = zlib.decompressobj()
            data.extend(d.decompress(packed, LIMIT-len(data)))
            if not d.eof or d.unused_data or d.unconsumed_tail:
                raise ValueError('invalid zlib stream boundary or PSIN exceeds output limit')
            previous_end = hi
    return data, checksum_psin(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('list', 'extract'))
    parser.add_argument('source', type=Path)
    parser.add_argument('--base', type=lambda x: int(x, 0), required=True)
    parser.add_argument('--ordinal', type=int)
    parser.add_argument('--out', type=Path)
    a = parser.parse_args()
    try:
        key, scope, rows = catalog(a.source, a.base)
        if a.action == 'list':
            info = {'archive': key, 'base': hex(a.base), 'native_toc_decoded': False, 'entries': rows}
        else:
            if a.ordinal is None or not 0 <= a.ordinal < len(rows) or a.out is None:
                raise ValueError('extract requires --ordinal in range and --out')
            data, crc = extract(a.source, scope, rows[a.ordinal])
            with a.out.open('xb') as f:
                f.write(data)
            info = {'out': str(a.out), 'bytes': len(data), 'CHKS': hex(crc)}
    except (OSError, ValueError, zlib.error) as exc:
        parser.exit(2, f'{parser.prog}: {exc}\n')
    print(json.dumps(info, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
