#!/usr/bin/env python3
"""Read RPF9 header geometry; does not decode the TOC or verify torrent pieces."""
import argparse
import csv
import json
from pathlib import Path
import struct

FIELDS = ('magic','A','B','C','u32_10','u32_14','u32_18','D','u16_20',
          'mode','N','u32_28','u32_2C','u32_30','u32_34','u32_38','u32_3C')
LAYOUT = struct.Struct('<8I2H7I')


def align16(size):
    return (size + 15) & ~15


def inspect(path, base, archive_end=None):
    with path.open('rb') as f:
        size = path.stat().st_size
        limit = size if archive_end is None else archive_end
        if base < 0 or not base + 64 <= limit <= size:
            raise ValueError('archive base or end is outside the file')
        f.seek(base)
        h = dict(zip(FIELDS, LAYOUT.unpack(f.read(LAYOUT.size)), strict=True))
        if h['magic'] != 0x52504639:
            raise ValueError('expected on-disk 9FPR')
        A, B, C, D, N, w = (h[k] for k in ('A', 'B', 'C', 'D', 'N', 'mode'))
        if not (0x40 <= D <= B <= A <= limit-base and not (A|B|C|D)&15):
            raise ValueError('metadata bounds or alignment do not fit the file')
        v = h['u32_2C'] & 255
        predicted = align16(N*(8+v)) if w == 1 and h['u32_2C'] in (0x90F01, 0x90F02) else None
        prefix = f.read(min(32, A-LAYOUT.size)).hex()
    warnings = []
    if predicted is not None and predicted != A-B:
        warnings.append('mode 1 size formula does not match')
    if w == 2 and h['u32_30'] == 16 and h['u32_2C'] in (0x90F01, 0x90F02):
        approx = align16(N*(8+2*v)+8)
    else:
        approx = None
    if (base == 0 or archive_end is not None) and 0 < C <= limit-base-A:
        footer = [limit-C, limit]
    else:
        footer = None
    return {'base': base, 'fields': h, 'metadata_ranges_absolute':
            [[base, base+0x40], [base+0x40, base+D], [base+D, base+B], [base+B, base+A]],
            'prefix_40_5F': prefix, 'possible_trailer_range_absolute': footer,
            'last_block_bytes': A-B, 'mode1_predicted_bytes': predicted,
            'mode2_heuristic_bytes': approx, 'warnings': warnings}


def check_corpus(root, table):
    totals = {'headers': 0, 'mode1_exact': 0, 'mode2_heuristic_exact': 0,
              'mode2_heuristic_total': 0}
    with table.open(encoding='utf-8-sig', newline='') as f:
        for r in csv.DictReader(f, delimiter='\t'):
            got = inspect(root/r['path'], int(r['base'], 16))
            for k in ('A', 'B', 'C', 'N', 'u32_10', 'u32_14', 'u32_18', 'u16_20', 'u32_28', 'u32_2C', 'u32_30'):
                if got['fields'][k] != int(r[k], 0):
                    raise ValueError(f'{r["path"]}+{r["base"]}: {k} differs from corpus')
            for src, dst in (('u32_1C', 'D'), ('u16_22', 'mode')):
                if got['fields'][dst] != int(r[src], 0):
                    raise ValueError(f'{r["path"]}+{r["base"]}: {src} differs from corpus')
            if got['prefix_40_5F'] != r['prefix_40_5F'] or got['warnings']:
                raise ValueError(f'{r["path"]}+{r["base"]}: prefix or size differs from corpus')
            totals['headers'] += 1
            if got['mode1_predicted_bytes'] is not None:
                totals['mode1_exact'] += 1
            if got['mode2_heuristic_bytes'] is not None:
                totals['mode2_heuristic_total'] += 1
                totals['mode2_heuristic_exact'] += got['last_block_bytes'] == got['mode2_heuristic_bytes']
    return totals


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('path', type=Path, help='RPF file or root folder with --check-corpus')
    p.add_argument('--base', type=lambda x: int(x, 0), default=0, help='absolute offset, e.g. 0xC4000')
    p.add_argument('--archive-end', type=lambda x: int(x, 0), help='optional absolute end of nested archive')
    p.add_argument('--check-corpus', type=Path, help='validate against the local header TSV')
    a = p.parse_args()
    try:
        r = check_corpus(a.path, a.check_corpus) if a.check_corpus else inspect(a.path, a.base, a.archive_end)
    except (ValueError, OSError, struct.error) as e:
        p.exit(2, f'{p.prog}: {e}\n')
    print(json.dumps(r, indent=2))


if __name__ == '__main__':
    main()
