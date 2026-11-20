# RPF9 format notes from local samples

These notes document byte-level recovery from local samples. The container specification is incomplete. Signature-based extraction (`salvage`) recovers accessible resources. Enumerating original entries depends on decoding the native index. Unknown fields require separate verification before a parser can treat them as zero, optional, or encrypted.

In this document, `u16le` and `u32be` are unsigned integers of the corresponding width and byte order. `f32le` is IEEE 754. Offsets are relative to the start of the object unless stated otherwise. Ranges are half-open. `align16(x) = (x + 15) & ~15`.

## Current results

| Area | Result |
|---|---|
| Archive coverage | All 22 local RPF files were examined. The pass found 4,373 candidate headers and 2,240,509 complete zlib streams. In 3,654 regions, the recognized object count and the candidate trailer both fit. |
| Header geometry | `A`, `B`, and `D` bracket the front regions. `C` helps locate trailers in verified samples. `N` matches the payload count in thousands of tested regions. The observed length of `[B,A)` constrains possible layouts. Field meanings inside that block are still unknown. |
| Native index | Original names, directory links, entry order, and the encoding of physical offsets and sizes remain unresolved. Literal searches, legacy RPF layouts, and the tested transforms produced no usable decoder. |
| Physical layout | Tested objects occupy sequential runs inside each container. Containers can nest. All 13 verified `data.rpf` children have exactly 1,115 object starts and byte-identical trailers. Their base-relative front metadata differs. §35 explains the conflicting report. |
| Mixed storage | Some resources combine compressed streams with raw system tables or graphics data. In a bounded follow-up on `levels.rpf`, resource-local pointers and lengths explained 16,801 of 16,850 nonzero gaps. The meanings of the recurring 16-byte records remain unknown. |
| Resource recovery | All 1,012 PSINs in the second `script.rpf` child pass CHKS. In one `story000.rpf` child, all 734 checked PSINs pass, including 26 that require raw fragments. Other recovered samples include XML, HMAP, UIHM, and SPS1. |
| Movies and images | All 515 Bink ranges in `movies.rpf` have verified boundaries. The tested mobipeg build decoded 13 of 14 video samples. Pillow rejected all 74 earlier JPEG candidates. |
| Audio | The ADAT walk confirms 7,160 banks and 1,603,123 nonempty `0x55` payload tags. Both extractors produced byte-identical outputs for all 50 multistream banks, and FFmpeg decoded them all. The Opus packets were remuxed without re-encoding. |

The per-archive report is `RPF_извлечённое/rpf9_payload_coverage.tsv`. Sections 1–34 give the evidence and reproduction commands. Section 35 reconciles the second agent's report with direct checks of the archives.

## 1. Which bytes are suitable for investigation

The local `.torrent` is bencode with a single top-level key, `info`; inside are `files`, `name`, `piece length`, `pieces`, and `private`. Under BEP 3, files form a logical concatenation in `info.files` order, not sorted path order. `pieces` is a sequence of 20-byte SHA-1 hashes; `info_hash` is SHA-1 of the original bencoded `info` subrange, not a hash of the entire download. In this copy, the exact `info` subrange is `[7,273882)`, `info_hash=465d6a0eac381e68ca77115f44f19f0c968b9590`. There are 108 files, 223,880,932,870 bytes, and 13,345 pieces of 16,777,216 bytes (the last is shorter).

| Fully verified RPF | Size, bytes | Piece numbers, zero-based | RPF SHA-256 |
|---|---:|---|---|
| `appdata0.rpf` | 8880 | 262 | `d9d4d2ddcd23e73a8862fbb5d5b53ca259e454f90319910d88f3a5c8da212535` |
| `common.rpf` | 6642192 | 1964 | `31dc6aa5394dd8cd3c83b90c9661f37325f22851cc6d5d7150e11041c7618ced` |
| `script.rpf` | 4358960 | 13307, 13308 | `c4d7e75f867c8527eee75135cd400e22f7c770c159c6bc2c3f80ec0f6e6a8dbb` |

Piece verification includes neighboring files when a piece crosses a boundary. If a neighboring range is unavailable, the result is `unverified`, not `failed`. If all bytes are readable but the SHA-1 does not match, the status is `failed`. `.fastresume` is an optional hint: in the local copy it did not mark piece 1964 or either `script.rpf` piece, although their SHA-1 hashes matched. A large file's visible size does not guarantee that all its bytes are present. For partial archives, retain verified ranges, do not replace gaps with zeroes, and do not use a missing signature in an unverified range as proof that the signature is absent. In the large archives, the initial ranges of `anim` (piece 0), `audio` (262), `movies` (10189), and `shaders` (13308), and the final ranges of `anim` (262), `audio` (1964), `levels` (10189), and `packs` (13307) were also confirmed; this does not confirm the other ranges.

## 2. RPF9 outer container

| Offset | Type | Observation |
|---:|---|---|
| `0x00` | 4 bytes | `39 46 50 52` = ASCII `9FPR` = `u32le 0x52504639` |
| `0x04` | `u32le` | `A`, a multiple of 16; a change in header region is observed after this position: in `script`, a gap before data begins |
| `0x08` | `u32le` | `B`, a multiple of 16, `B < A` in all five checked headers |
| `0x0C` | `u32le` | `C`, a multiple of 16; candidate length of a terminal region. Its relation to the end is verified for the specific samples below; semantics are undetermined |
| `0x10`, `0x14` | `u32le` | changing words; purpose unknown |
| `0x18`, `0x1C` | `u32le` | `0x47A00`, `0x150` in all five checked headers; purpose unknown |
| `0x20`, `0x22` | `u16le` | observed pairs `(3,1)`, `(6,1)`, `(7,1)`, `(6,2)` |
| `0x24` | `u32le` | candidate entry count: in nested archive `script+0xC4000`, it equals the number of recovered PSINs. Do not use it as proof of complete extraction from other archives |
| `0x28` | `u32le` | changing word, correlates with some variants of `0x20`; purpose unknown |
| `0x2C` | `u32le` | `0x00090F01` or `0x00090F02` in checked headers |
| `0x30` | `u32le` | `0x10` in checked headers |
| `0x34..0x40` | 12 bytes | zeroes in checked headers |
| `0x40..` | bytes | unread region; the first 32 bytes here are identical across the five headers |

| Sample | Base in RPF | A | B | C | `u32le[0x24]` |
|---|---:|---:|---:|---:|---:|
| appdata0 | `0x0` | `0x360` | `0x340` | `0x40` | 2 |
| common | `0x0` | `0x9580` | `0x8520` | `0x3A10` | 418 |
| script, outer | `0x0` | `0x260` | `0x240` | `0x80` | 2 |
| script, nested #1 | `0x1000` | `0x480` | `0x460` | `0x40` | 2 |
| script, nested #2 | `0xC4000` | `0x11F10` | `0xEF90` | `0x62B0` | 1012 |

Boundaries, all absolute in the parent file. `EOF−C` for `appdata0` (`0x2270`) and outer `script` (`0x4282B0`) starts with the same 16 bytes, `b7da7c1d963787c846fe15fd0e243364`. The same byte string is at `script+0xC3B50`, immediately after aligning the end of the last zlib stream in the block at `0x53000`. If the `C` region of the first nested archive starts there, its end is `0xC3B90`, not `0xC4000` (zeroes precede the next header). In `common`, `EOF−C=0x652000`, immediately after the last recovered SPS1 and a gap; the same byte string is absent there. For the second nested `script`, position `0x422000` is a candidate from the formula `0x4282B0−0x62B0`; the interpretation of terminal regions remains a hypothesis. Do not search for an archive end only by the shared 16-byte string, and do not assign boundaries based only on the next signature.

### Recovering zlib streams

1. Scan offsets divisible by 16 within verified ranges. Check the RFC 1950 header: `CMF & 0x0F == 8`, `CMF >> 4 <= 7`, `(256*CMF + FLG) % 31 == 0`. This profile has no dictionary: skip `FDICT=1` as unsupported, not corrupt.
2. Decompress **one** stream with input/output limits. Require `eof` and successful Adler-32 verification by the decoder. In Python, with a single input call, `stored_end = read_end - len(unused_data)`; do not accept `unconsumed_tail` as a valid end. Store the offset and compressed size separately for each stream.
3. For an incomplete logical resource, try the next stream at `align16(stored_end)` if the previous stream decompressed to `0x10000` bytes. This is a confirmed sample pattern, not a universal indicator of file length. Do not append the next stream to an already complete PSIN/XML just because the first produced 64 KiB.
4. Confirm the logical resource's end using its own format: complete XML parsing, PSIN graph and CHKS, exact map size, or complete SPS1 list. Preserve an unknown block without such verification with the label `heuristic_boundary`.

Result: `common` contains 954 valid zlib streams, 679 of them 64 KiB; 259 complete XML files (257 with a declaration, including BOM, and two more without a declaration: `0x220000` and `0x442000`). `script` contains 1057 streams: 1015 for 1012 PSIN instances and 42 for the block at `0x53000`. There are 869 distinct PSINs by original decompressed content; when extracting, preserve all 1012 instances and do not deduplicate by default. No aligned zlib streams were found in `appdata0`, although the archive was fully checked. The scanner does not extract entries stored without zlib or using other storage methods.

## 3. PSIN

### Header and main sections

* `+0x00`: ASCII `PSIN`; `+0x04`: `u32be extension_start` — offset of `LSCH`, not file size.
* Starting at `+0x08`: 16-byte descriptors: `u32le id`, `u32le size`, eight `0xAA` bytes. Then eight `0x70` bytes and immediately the first section's bytes. Sections follow descriptor order. The next section starts at `align16(start+size)`; padding is zero in checked files. `LSCH` begins after the last aligned section.

| ID | Data | Number of files out of 1012 |
|---|---|---:|
| `0x00000016` | Table of 8-byte values; for export, store `u64le` as hex or a decimal string to preserve precision in JSON | 1010 |
| `0xAEB623D8` | Table of `f32le` triples, 12 bytes per record | 855 |
| `0x85B4C631` | 16-byte attributes | 1012 |
| `0xDE396FE2` | Metadata, 80 bytes; `u32le[0x00]` and `[0x04]` equal the attribute count, `[0x40]` and `[0x44]` equal the node count. Do not treat `[0x30]` and `[0x34]` as another copy of the node count | 1012 |
| `0x01393259` | 16-byte nodes | 1012 |

Attribute: `0x00 u32le id0`, `0x04 u32le id1`, `0x08 u32le raw_value`, `0x0C u8 type`, `0x0D u8 flags` (always 1), `0x0E u16le reserved` (always 0). The semantics of the two identifiers are not final. Types in the checked set:

* `1`: zero-based index into the triples table (`13257/13257` within the table).
* `2`: `f32le` bits (`15277` records).
* `3`: 32-bit integer (`14739`); preserve the original bits, since signedness is not proven by this corpus alone.
* `4`: boolean `0/1` (`10` records). Observed values also fall within valid `u64` indices in these files; the value range alone is insufficient to prove the type.
* `5`: zero-based index into the 8-byte values table (`20005/20005` within the table); the purpose of the values is unproven.

Node: `0x00 u32le id`; `0x04 u16le parent`, `0x06 u16le first_child`, `0x08 u16le next_sibling`, `0x0A u16le prev_sibling`, `0x0C u16le first_attr`, `0x0E u8 attr_count`, `0x0F u8 flags`. Indices are zero-based; `0xFFFF` means no reference. If `attr_count>0`, use range `[first_attr,first_attr+attr_count)`. Ranges and reciprocal links were checked for all 20,379 nodes: the first child names this node as its parent, and adjacent sibling nodes point to each other and share the same parent. The parser also limits depth and checks for cycles. The purpose of the final byte is undetermined.

### Extension sections

After `extension_start`, each section is `tag[4]`, `u32be size_including_header`, `payload[size−8]`. The next begins exactly at `offset+size`, without rounding. In all 1012 instances, the order is `LSCH → LENM → LSNS → LGID → LRSC → CHKS`. Preserve unknown sections whole; do not assume the extension region is aligned to 16 bytes. In `LSNS`, `u32le` at `+8` is 1, `u32le` at `+12` is `0x101`, and `u32le` at `+16` is `size−20` in all samples; the payload format that follows is unknown. `LGID` is empty throughout the corpus. The confirmed internal structure of `LSCH`, `LENM`, and `LRSC` is given in section 5.

### CHKS: exact formula

The final 20 bytes are: `CHKS`, `u32be 20`, `u32be full_size`, `u32be checksum`, `zppp`. Zero **only** the eight bytes `full_size` and `checksum`, i.e. `[len−12,len−4)`. Do not zero the section length or the extension pointer in the header. Hash all bytes, including labels, using a **signed 8-bit input**: `0x80..0xFF → −128..−1`. Ordinary unsigned JOAAT does not work here.

```text
h = 0x3FAC7125
M = 0xFFFFFFFF
for raw in file_after_zeroing:
    x = raw if raw < 128 else raw - 256
    h = (h + x) & M
    h = (h + (h << 10)) & M
    h = h XOR (h >> 6)
h = (h + (h << 3)) & M
h = h XOR (h >> 11)
h = (h + (h << 15)) & M
```

Right shifts are logical (`h` is `u32`). The result matched 1012/1012; this check does not establish provenance. Test vectors: `script+0xD6000` → `5392`, CHKS `0x99FE669F`, SHA-256 `3fda109bb089f9b394b03e8daf96848bed89f1742ad9b5ef4d01671068448f27`; `script+0xD7000` → `9600`, `0x32E90989`; `script+0x10A000` → `992`, `0xE73AE32D`.

## 4. Other resources

### HMAP map — physical signature `PAMH`

`common+0x12B000`, one stream ending at `0x12E239`, 48,588 bytes. Start: `+0x00 PAMH`, `+0x04 u32le 0x201`, `+0x08 u32le 0x1BBC8CED` (unknown field), `+0x0C u16le width=144`, `+0x0E u16le height=164`. Six `f32le` values from `+0x10`: `(-13312,−9728,−302,5120,11264,404)` — candidate XYZ bounds. `+0x28 u32le payload_size=48544`, length starting at `+0x2C`. There are 164 `<HHI>` descriptors there; all equal `(0,144,144*row)`. Exactly `2*144*164` bytes follow. This is two bytes per cell **by size**, while their meaning, order, and dequantization are unconfirmed. Directly interpreting adjacent `u8` values as ordered `(min,max)` produces 3555 inversions; do not present that as a proven schema.

### UIHM maps — physical signature `MHIU`

`common+0x144000` and `common+0x16E000`, 19 streams each; lengths 1,189,171 and 1,210,476 bytes. `+0x04 u16le=1`, `+0x06 u16le width=2048`, `+0x08 u16le height=2048`, `+0x0A u16le=0`; `+0x0C f32le[6]` are identical for both maps (candidate XYZ bounds); `+0x24 u32le payload_size`, `+0x28 u32le unknown`. Total size is `0x2C+payload_size`.

From `0x2C` there are `height` 8-byte `<HHi>` descriptors: `first_col`, `stored_count`, `row_bias`. The byte pool starts at `0x2C+height*8` (`0x402C`). If `stored_count=0`, the row is not represented; do not use `row_bias`. For `first_col <= x < first_col+stored_count`, the value is `pool[row_bias+x]`; distinguish missing cells from numeric zero. In both maps, `first_col+stored_count<=width`, and the beginning of each nonempty row (`row_bias+first_col`) equals the sum of stored values in previous rows. The pools contain 1,172,743 and 1,194,048 bytes. The physical units of the values are unknown.

### SPS1 catalog

`common+0x650000`, one stream ending at `0x651B8F`, 43,791 bytes. `+0x00 SPS1`, `+0x04 u32le count=858`. Starting at `+0x08` are unpadded records: `u8 path_len`, `path[path_len]`, `u8 technique_len`, `technique[technique_len]`, `u8 flags`. Lengths include a single terminal `NUL`. All 858 records were read exactly to the end of the resource. First record: `1Structure/be_standard.sps`, `normal`, `0`. The meaning of `flags` bits is unknown. Paths in SPS1 are resource fields, not RPF9 entry names.

### Unidentified index in script

`script+0x53000`, 42 streams, the last ending at `0xC3B45`; 2,702,384 bytes decompressed. SHA-256 `592afd44c211a8d07ca7f90ef3f8659d272b2d271b5a9a85ed735a5ed80c3c82`. `+0x14 u32le count=30683`, `+0x28 u64le offsets[count]`. Offsets are relative to the start of the decompressed block, strictly increasing, and all divisible by 16. The first, `0x3BF00`, equals the end of the array; the last is `0x293C00`. Individual records contain candidates for internal references, for example `u32le[0x3BF10]=0x3BF28`; this reference does not match the record start (the next record starts at `0x3BF20`). Record types and purpose are unknown; this is not a proven RPF9 index.

## 5. Additional checks of containers and resources

### Checked boundaries and a fourth complete archive

SHA-1 was recomputed for all 23 distinct torrent chunks containing the first or last bytes of `.rpf` files: 14 matched, 3 did not, and 6 lacked sufficient bytes. A mismatch applies to the entire chunk, including neighboring files, and does not automatically implicate the RPF header. This check does not cover the middle of large archives. It added a fourth fully checked archive: `pack/core_licenses/core_licenses_data.rpf`, 13,536 bytes, entirely within chunk 11610, SHA-256 `6ca2574aa699b4583f110c6b3e3ae605a6a0ca8c36d65769f76e3beb58e93751`.

* Outer `core_licenses_data`: `A=0x330`, `B=0x300`, `C=0x90`, `u32le[0x24]=4`; start of the presumed terminal region is `EOF−C=0x3450`. The last of the three recovered XML streams lies immediately before it and ends at `0x3445`; the next 11 bytes are zero.
* Nested `core_licenses_data`: base `0x1000`, `A=0x320`, `B=0x300`, `C=0x50`, `u32le[0x24]=2`. The shared 16-byte marker `b7da7c1d963787c846fe15fd0e243364` is at `0x2E00`; `0x2E00+C=0x2E50` is exactly the first byte of the zero gap before the next stream at `0x3000`. This is an independent example consistent with the hypothesis about `C`, but it does not prove a universal marker or its purpose.
* In the first nested `script`, the entire interval `[0xC3B90,0xC4000)` is zero. In the second, the last stream at `0x41D000` ends exactly at `0x42177D`, then the entire interval `[0x42177D,0x422000)` is zero. This confirms separation of regions at the presumed boundaries, but the terminal-region records themselves have not yet been read.
* For `anim`, `audio`, `common`, `data`, `levels`, `movies`, and `packs`, the calculation `EOF−C` gives an offset aligned to `0x1000` and a region with nonzero data in the confirmed chunk. Other confirmed examples (`appdata0`, `core_licenses_data`, `script`, `shaders`, `textures`) disprove a rule requiring `0x1000` alignment. Do not round `EOF−C`.

In `core_licenses_data`, the streams `[0x3000,0x3196)`, `[0x31A0,0x32A7)`, and `[0x32B0,0x3445)` yield valid XML of 763, 455, and 786 bytes. The references form a chain: `SSetupData/datFile` names `content.xml`; `CDataFileMgr__ContentDef` refers to `$(root)/changesets/ccs_core_licenses_licenses.meta`; `CDataFileMgr__ContentChangeSetDef` with the corresponding ID refers to `$(target)/data/licenses/license_additional_content.rpf`. These are candidates for matching to blocks, not names read from the RPF. The file has no direct matches, in either byte order, for ordinary 32-bit JOAAT with zero seed from `content.xml`, `setup.xml`, `setup2.xml`, the changeset name, or the nested RPF name (the last two strings were checked both with and without their relative directory). The three XML files and one nested `9FPR` agree with the number 4 in the outer container header.

### PSIN: schema fields and limits of LSNS analysis

Offsets below are from the start of the section and include its 8-byte header. Across 1,012 instances, `LSCH` has only two distinct byte-for-byte patterns: 472 bytes in 157 instances without the triple table, and 552 bytes in 855 instances with the triple table.

* Starting at `+0x08` are three or four `0x20`-byte descriptors, in the order `0xDE396FE2`, `0x85B4C631`, `[0xAEB623D8]`, `0x01393259`. In a descriptor, `+0x00 u32le` matches the section ID, `+0x10..+0x18` is eight `0xAA` bytes, and `+0x18 u32le` is the confirmed record size: `80`, `16`, `[12]`, `16`. Preserve the other fields as raw bytes. After the descriptors comes an identical region of five 32-byte records, followed by 16-byte tables. The variant with the triple table adds one descriptor and three 16-byte records. In the final table, the node-field offsets `0,4,6,8,10,12,14,15` match the established layout of a 16-byte node.
* `LENM` has one byte-for-byte pattern across the entire corpus. `+0x18 u32le=6`; starting at `+0x20` are six `<IIII>` records: `(ordinal,0,id,0)` with `ordinal=0..5`. The semantics of `id` are not established. `LGID` is empty in all 1,012 instances. `LRSC` has three 88-byte patterns; in all of them, `+0x08` starts with ASCII `0DRP`, and `+0x0C u32le=0x7A`.
* In `LSNS`, the length after the `0x14`-byte header is always a multiple of 16 (`1012/1012`), with a maximum of 19,392. Of 869 unique payloads, only 208 unique initial 16-byte blocks occur, and no aligned 16-byte blocks repeat within one payload. Empirical byte entropy over the union of unique payloads is about 7.999 bits/byte. These are properties of the observed representation; they prove neither a particular codec, nor encryption, nor content.

### HMAP: two planes instead of interleaved bytes

After the string descriptors at `0x54C` are two consecutive `u8` planes of `144×164=23616` bytes. The first begins at `0x54C`, the second at `0x618C`. In row-major storage, use `i=row_offset+(x-first_col)` in each plane. With this split, `plane0[i] > plane1[i]` for all 23,616 cells, with differences from 1 to 146. This strongly supports a pair of “upper and lower height codes,” unlike a byte-interleaved format. Do not calculate heights from these codes without confirming the scale and filler values: for example, `(110,0)` occurs in 12,986 cells.

## 6. Checking the RPF9 directory

The RPF9-native directory has not yet been decoded. The files `RPF_извлечённое/toc_regions.json` and `RPF_извлечённое/script_observed_entries.tsv` are, respectively, a map of header regions and a byte-recovered listing of `script.rpf`, not a dump of the original directory. All TSV offsets are absolute, the end is exclusive, and game names have not been recovered.

We checked 131 `9FPR`-signature candidates for which the entire region `[base,base+A)` was inside a range covered by a confirmed torrent-chunk SHA-1. In `[+0x40,+0x50)`, all have the same 16 bytes; a difference first appears at `+0x50`. A bytewise scan of these regions after `+0x40` found no complete zlib stream producing at least 16 output bytes. The known offsets of four objects in the outer `core_licenses_data` and two child `script` archives, as well as 1,012 PSINs, were not found there as ordinary 32-bit pointers (LE/BE, absolute or base-relative, in bytes or in units of 16/512/4096 where applicable). This constrains simple format hypotheses but does not prove a particular encoding method.

`A−B` often equals `align16(9*N)` or `align16(10*N)`, where `N=u32le[+0x24]`, but this is not universal. For example, for `script+0xC4000`, `N=1012`, `A−B=0x2F80`, which does not equal `align16(12*N)`. Do not treat `A−B` as a fixed-record table without further confirmation.

| `script.rpf` base | What was entered in the listing | Supporting evidence |
|---:|---|---|
| `0x0` | Two child RPFs at `0x1000` and `0xC4000` | Signatures and headers; end boundaries inferred from neighboring regions |
| `0x1000` | Unread region `[0x2010,0x52FE0)`, then indexed block `[0x53000,0xC3B45)` | First: from first to last nonzero byte; second: 42 checked zlib streams. Do not automatically treat the first as one file, a script, or confirmed ciphertext |
| `0xC4000` | 1,012 PSINs, from `0xD6000` to the end of the last stream at `0x42177D` | 1,012 CHKS; `N=1012`; all gaps between streams and up to the presumed terminal region at `0x422000` are zero |

The index of 30,683 offsets belongs to the address space of the decompressed block. In 12,335 records marked `qqqq` at record offset `+0x14`, the 32-bit field at `+0x10` points exactly to `start of next record + 8` within the block. These are verified internal references, not file offsets in `script.rpf`.

## 7. Comparison with RPF8

Reference for the RPF8 layout: source files [Swage, rpf8.h](https://github.com/0x1F9F1/Swage/blob/master/src/games/rage/rpf8.h) and [Swage, rpf8.cpp](https://github.com/0x1F9F1/Swage/blob/master/src/games/rage/rpf8.cpp). This describes the variant read by `LoadRPF8`, not an assumption about RPF9 internals.

| Field | RPF8 in Swage | RPF9 in local samples |
|---|---|---|
| On-disk signature | `8FPR`, `u32le 0x52504638` | `9FPR`, `u32le 0x52504639` |
| `+0x04` | `EntryCount` | `A`, boundary of the initial region in checked samples; not an entry count |
| `+0x08` | `NamesLength` | `B`, aligned field with `B<A`; meaning unknown |
| `+0x0C` | Two `u16le`: `DecryptionTag` and `PlatformId` | `u32le C`; strongly consistent with terminal-region length, but its contents have not been read |
| Count | `u32le` at `+0x04` | Candidate `u32le` at `+0x24`; example `script+0xC4000`: 1,012 PSINs |
| Directory | After the 16-byte header and 256-byte signature; from `+0x110`, 24 bytes per entry; decrypted when needed | Neither directory start nor entry layout after possible decoding has been established |
| Compression | Entry ID: 0 — uncompressed, 1 — raw DEFLATE, 2 — Oodle | zlib streams with RFC 1950 wrapper are confirmed for some entries; they are sometimes joined across a 16-byte boundary and yield 64 KiB per intermediate stream. Codec ID table is unknown |

Check: applied the RPF8 array-end formula `0x110 + 24*u32le[+0x04]` to six local headers. In all six it ended beyond `A`; in `appdata0.rpf` (end `0x5210`, whole file `0x22B0`) and `core_licenses_data.rpf` (`0x4D90`, whole file `0x34E0`) it even ended beyond EOF. Then `u32le[+0x24]` was substituted for the count, and the 24-byte entry bit fields were checked without decryption at `+0x110`, `+0x150`, and `+0x160`: no entry under any combination yielded a nonzero stored size and range wholly contained between `A` and its archive end. Positive control: the same code correctly read a synthetic RPF8 entry with address `0x200` and size 16. This rules out a simple magic substitution or moving the start of an open table; the test does not rule out similarities in decoded entries. This comparison does not establish that RPF9 uses RSA/TFIT.

## 8. Refined RPF9 header layout

Corpus `RPF_извлечённое/rpf9_header_corpus.tsv`: 193 candidate headers from torrent-confirmed ranges. For each, the entire initial region `[base,base+A)` is available; selection required the signature, alignment of `A/B/C`, reasonable boundaries, and a zero tail in the fixed header at `+0x34..+0x3F`. Therefore, those final zeros are a selection criterion, not an independent check of universality. 131 headers were in the initial sample; 62 were found by a full `9FPR` search in the same checked ranges and were not used to derive the formula below.

| Offset | Confirmed observation |
|---|---|
| `+0x10`, `+0x14` | Meaning unknown. No matches for CRC-32 or Adler-32 in either byte order were found for the regions below, including `[B,A)` and `[u32le(0x1C),B)`. This does not rule out other checksums or input ranges. |
| `+0x18` | `u32le` is not constant: observed values `0x47A00`–`0x87A00` in steps of `0x10000`; the low 16 bits are `0x7A00` in all 193. |
| `+0x1C` and `+0x30` | Both `u32le` values are multiples of 16. Typical pair `(0x150,0x10)` occurs 184 times. In 192 of 193, the difference is `0x140`; the exception is `shared_minigame000.rpf+0x273E3000`, pair `(0x380,0x180)`, difference `0x200`. In all 193, `0x140 <= u32le(0x1C) < B < A`. These are candidates for a boundary and sub-block length; their role is unconfirmed. |
| `+0x22` | Observed `u16le` values: `1` (151), `2` (39), `6` (1), `7` (2). Do not apply the size formula below to modes `2/6/7`. |
| `+0x24`, `+0x2C` | `N=u32le(0x24)` is a candidate count; `u32le(0x2C)` takes `0x90F01` for observed `N=1..231` and `0x90F02` for `N=260..16384`. There are no samples with `N=232..259`. A threshold around 256 and interpreting the low byte as index width are hypotheses, not rules. |
| `+0x40..+0x4F` | All 193 contain the same 16 bytes `5f08daabecbdd40258c3516807024c24`. The next 16-byte block has four variants, with frequencies 185, 6, 1, and 1. Do not substitute this block for the primary signature. |

**Formula for `u16le(0x22)=1`:** for `v = u32le(0x2C) & 0xFF` and `v ∈ {1,2}`, `A−B = align16(N*(8+v))`. In other words, 9 bytes per element before alignment for `0x90F01` or 10 for `0x90F02`. It matched 108/108 in the initial sample and 43/43 in the additional sample. For example, `common.rpf`: `N=418`, `v=2`, `A−B=0x1060`; `audio.rpf`: `N=103`, `v=1`, `A−B=0x3A0`. This lets us predict the size of the last sub-block in the initial region and check the count hypothesis. It still does not let us read names, offsets, or flags inside the sub-block. In `script.rpf+0xC4000`, the value at `+0x22` is `2`, so the formula does not apply.

For an experimental parser, it is safe to read fixed fields and preserve the ranges `[0x40,u32le(0x1C))`, `[u32le(0x1C),B)`, and `[B,A)` as raw data, and use the formula for diagnostics rather than forcibly repairing headers. The native directory is still not decoded in these data.

## 9. Checking the data boundary and mode 2

For the same 193 checked headers, the first nonzero byte after `base+A` was measured; reading was limited to the range covered by a confirmed torrent chunk. In all 193 cases, the byte fell on a 16-byte boundary. Only 26 begin exactly at `A`, and only 53 exactly at `align_up(A,0x1000)` relative to the base. Thus, `A` separates initial metadata, but does not by itself give the address of the first entry; hard page rounding does not work. At the first nonzero byte, a complete zlib stream was verified in 153 cases, `9FPR` occurs in 14, `ADAT` in 3, `KB2n` in 1, and the type is undetermined in 22. `rpf9_header_payload.tsv` records offsets, zero counts, and leading bytes. This does not prove that the first nonzero byte is always the start of an archive entry: a raw object may itself begin with zeros.

For `u16le[+0x22]=2`, `u32le[+0x30]=0x10`, the approximation `A−B = align16(N*(8+2*v)+8)` works, where `v = u32le[+0x2C] & 0xFF`. It matched 34 of 35 samples, including `script.rpf+0xC4000`: `N=1012`, `v=2`, result `0x2F80`. Counterexample: `levels.rpf+0x1C000`, `N=8`, `v=1`, actual `A−B=0x90`, predicted `0x60`. The counterexample was checked further: exactly eight PSINs were recovered between `0x1D000` and `0x4C219`, and all had matching CHKS. The last two occupy 13 and 2 zlib streams, respectively. After the last stream there are 7 zeros up to `0x4C220`, then a nonzero region of total length exactly `C=0x270` up to `0x4C490`; from there to the next `9FPR` at `0x4D000` is a zero gap. This confirms the local split into data and terminal region and rules out a count error as an explanation for the formula mismatch in this sample. In all four mode-2 samples with `u32le[+0x30] != 0x10`, the formula did not match. This is a diagnostic hypothesis, not a rule for reading. The middle `B−u32le[+0x1C]` is not fixed even among 13 similar headers from `data.rpf` with the same `N=1115`, mode, and size of `[B,A)`; within this group it varies from 65,024 to 65,120 bytes.

A fixed-header reader that can be checked is in `RPF_извлечённое/rpf9_header_inspect.py`. From the archive directory:

```sh
python3 RPF_извлечённое/rpf9_header_inspect.py script.rpf --base 0xc4000 --archive-end 0x4282b0
python3 RPF_извлечённое/rpf9_header_inspect.py . --check-corpus RPF_извлечённое/rpf9_header_corpus.tsv
```

It shows four metadata ranges and a possible tail based on `C`, if the archive end is known. For a nested object, specify the end explicitly so the program does not treat the parent EOF as the nested object’s end. It does not read native directory entries or replace torrent-chunk verification.

## 10. Checking the trailer region against real entries

Of 42 headers followed by a detected PSIN start, 14 could be walked through all `N` entries consecutively, with CHKS verification, to obtain a plausible trailer boundary independently of the next header's start. These contain 2,262 PSINs in total. They are targeted examples with homogeneous content, not a universal way to read RPF. Do not treat the other 28 candidates as damaged: the method stops at an entry of another kind or beyond the confirmed range.

- The presumed trailer begins at the first nonzero byte after the last stream. In 10 archives this is `align16(last_stream_end)`; in 4 it is `align_up(last_stream_end,4096)`. A block of length `C` ended with a nonzero byte, followed by a zero gap or a known nested-archive boundary. A particularly convenient cross-check is `data.rpf+0x690000`: 181 PSINs, last stream ends at `0x88E291`, presumed trailer `[0x88F000,0x8909B0)`, next header at `0x891000`.
- We checked whether additional zlib streams explain the size of `[B,A)` in mode 2 with `u32le[+0x30]=0x10`. The adjustment `A−B = align16(N*(8+2*v)+max(8,4*extra_streams))` matched 4 of 5 samples, including the anomaly in `levels.rpf`, but was contradicted by `story000_restofworld.rpf+0x3AF5000`: two PSINs, four additional streams, actual `0x20`, predicted `0x30`. A simple universal correction based on stream count does not work.
- In `[D,B)`, `[B,A)`, and the presumed trailer, we checked for known physical PSIN offsets and stored lengths as ordinary `u32le/u32be`. For relative addresses, units of 16, 512, and 4096 were also checked where division was exact. Of 252 set comparisons, only one produced a numeric match: one of 816 absolute addresses in `[D,B)` at `data.rpf+0x9EF3000`. There were none in trailers or `[B,A)`. This does not confirm open tables of such numbers; packed or transformed fields remain possible.

Evidence is in `RPF_извлечённое/rpf9_psin_archive_geometry.tsv` (14 archives), `rpf9_psin_observed_entries.tsv` (separate absolute intervals for all 2,262 PSINs; physical order, not native order), and `rpf9_psin_metadata_integer_tests.tsv` (offset and length comparisons). Treat all trailer boundaries not obtained from the external EOF as inferred from observations, not as fields from the native TOC.

## 11. Control series: identical trailer with different data layouts

In the fully checked `data.rpf`, we found a series of 13 nested objects with `N=1115` and `C=0x74A0`. Each has exactly `0xB60` zero bytes before the next `9FPR`. The trailer candidate begins on a 4 KiB boundary, immediately after the zero gap following the last completed zlib stream. The end of that stream was checked separately in all 13. All thirteen trailers, each 29,856 bytes long, are **byte-for-byte identical**, SHA-256 `241aa550b916d962689b57c4ae52588780c32eb30c7443ac720c8cda8d7a3e41`. Meanwhile, `[D,B)` and `[B,A)` each have 13 different SHA-256 values, and the first large zlib stream beginning at `base+0x16010` has 13 different relative end positions. Therefore, the identical trailer alone cannot contain an unambiguous table of the exact physical boundaries of these streams. It may hold shared names or other common data; the trailer's purpose has not been established. See `RPF_извлечённое/rpf9_data_series_verified.tsv`.

The first large streams in this series produce 131,072 bytes in 6 samples and 196,608 in 7. All were completed with zlib verification. Therefore, a general RPF scan must not cap stream output at 128 KiB or assume 64 KiB is the only intermediate-block size. Earlier counts from the general scanner capped at 128 KiB are lower bounds. Do not join large streams based only on size.

A separate check of trailer boundaries on 20 objects with independent or previously inferred ending boundaries found that the rule “4 KiB for `C>=0x1000`, otherwise 16 bytes” matched in 18 cases. The exceptions are `audio.rpf` (`C=0xDD0`) and `shared_minigame000.rpf` (`C=0xC10`): both trailers begin after a longer zero gap, on a 4 KiB boundary. The threshold `C>=0x800` matches all 20 and is consistent with the 14 PSIN samples above, but it is fitted to this sample; in eight of 20, the preceding object's end is only the position after its last nonzero byte, not a confirmed zlib end. Do not use the threshold as a hard rule. See `RPF_извлечённое/rpf9_known_end_trailers.tsv`.

## 12. Catalog prototype and available decoder checks

At fixed starts in three metadata regions (`[0x40,D)`, `[D,B)`, `[B,A)`) and offsets of 4, 8, 16, and 32 bytes, we tried raw DEFLATE, zlib, gzip, XZ, and bzip2. In 13,835 attempts, no decoder read an entire region. The only partial raw DEFLATE result consumed 9 of 544 available bytes and yielded no recognizable structure. Other offsets, formats, dictionaries, block packing, or transformations remain possible. A search in local `Catalog000.bin`, `gameos.xvd`, and DLLs for literal `9FPR`, `RPF9`, `fiPackfile`, and some UTF-16LE variants found no open implementation. `Catalog000.bin` has a PE wrapper without an entry point or code section; this does not prove that an implementation is absent from other files or inside the container.

`RPF_извлечённое/rpf9_recovered_catalog.py` reads the recovered PSIN catalog from TSV, not the native TOC. It supports `list` and `extract` for specific bases of 14 nested objects. During extraction it checks exact stream boundaries and CHKS; it accepts a saved torrent-range map as input and does not recalculate SHA-1 pieces. It does not overwrite an existing output file. It does not assign original names.

```sh
python3 RPF_извлечённое/rpf9_recovered_catalog.py list script.rpf --base 0xc4000
python3 RPF_извлечённое/rpf9_recovered_catalog.py extract script.rpf --base 0xc4000 --ordinal 0 --out chosen.psin
python3 -B RPF_извлечённое/test_rpf9_recovered_catalog.py
```

## 13. Reproducible checks for metadata transformations

No transformation into native records has been established. The checks below are negative results for specific models, not proof of encryption or that the format cannot be decoded.

- In 579 header regions, we made 13,559 attempts each for raw LZ4, Zstandard, LZO1X, and Brotli: start offsets 0, 4, 8, 16, and 32; end truncations of 0, 4, 8, 12, and 15 bytes; output capped at 256 KiB. No complete decoding produced at least 16 bytes. Control inputs generated by the LZ4 and Zstandard compressors decoded successfully. `rpf9_extended_codec_trials.tsv` contains rows only for successful attempts, so it has only a header.
- For PSINs with known physical addresses and lengths, we checked 32-bit fields with a single constant XOR or modular addition modulo 2³². We used forward and reverse physical order, LE and BE, steps of 4–96 bytes, and starts within the first 256 bytes of each region. Relative addresses in units of 16, 512, and 4096 were also checked where all addresses divide exactly. Across 18,382,224 hypotheses, there were no four consecutive matches; a synthetic example with step 10, phase 5, and XOR `0xDEADBEEF` was detected. Different record order, variable step, bit packing, and other transformations are not ruled out. Table: `rpf9_word_transform_trials.tsv`.
- For 74 mode-1 headers with `N>=32`, we tested whether the extra one or two bytes per entry in `[B,A)` form an open permutation of ordinal numbers `0..N−1` or `1..N`. We tested interleaving with eight-byte records, a separate index table before or after the rest of the block, two byte planes, LE/BE, and every distribution of alignment bytes at the ends. None of the 6,939 layouts yielded a permutation. This does not rule out another meaning for the field, repeated indices, or an encoded representation. Table: `rpf9_ordinal_layout_trials.tsv`.

To reproduce: from the source-archive directory, run the three `.py` files in `RPF_извлечённое/probes/`; the codec test uses `liblz4.so.1`, `libzstd.so.1`, `liblzo2.so.2`, and `libbrotlidec.so.1`. Criterion for a future decoder: derive addresses and lengths from metadata bytes, without TSV or signature scanning, for a held-out archive; compare them with independently verified ranges; separately verify file boundaries and stream segmentation without conflating these layers. A useful new lead would be a native reader for this exact build or a pair of “encoded block → records read” examples; neither has been found yet.

## 14. Comparing streams across 13 data.rpf copies

This early pass capped each zlib stream's input at 512 KiB. Section 32 accounts for all 26 streams it missed.

We compared decompressed and stored contents byte-for-byte by SHA-256, separately from physical address. The range `[A, candidate_footer_start)` of each copy was scanned. Aligned zlib streams with a valid EOF were accepted; limits were 512 KiB input and 4 MiB output. In all 13 copies, the last detected end matched the previously verified boundary. In each copy, two additional zlib-header candidates did not reach EOF within the chosen limits; this does not prove that there are two more streams there.

- We found 14,628 nonempty streams and 14,512 distinct decompressed-content SHA-256 values. 56 contents occur in multiple copies, and only the first PSIN is identical across all 13. One stream does not necessarily correspond to one archive file: `N=1115`, while there are 1,122–1,133 streams. Concatenating all adjacent streams separated only by 16-byte alignment yields `N` in just one of the 13 copies. The largest confirmed stream produces 458,752 bytes (448 KiB).
- We found 55 matches that are unique within each copy where they occur, have identical **stored** bytes, and lie at different relative offsets. For example, the stream with SHA-256 of decompressed bytes `afed0288260340fc39944cedcfe63844d0ccbe0d765778f411944272ddb429e6` produces 1,072 bytes from 239 stored bytes and occurs in 12 copies at 12 different offsets. It is useful as an address-checking anchor, but does not prove a name or logical-record boundary.
- Across all 78 pairs, we compared sets of aligned 16-byte metadata blocks. Every pair shares two blocks in `[0x40,D)` and all 1,866 trailer blocks. In `[D,B)` and `[B,A)`, there is not a single identical block, even when comparing different positions within the corresponding region. These regions contain no open repetition of 16-byte records for the moved streams; this does not prove any specific transformation or encryption.

Results: `RPF_извлечённое/rpf9_series_members.tsv`, `rpf9_series_shared.tsv`, `rpf9_series_pairs.tsv`, `rpf9_series_scan_summary.tsv`. Reproduce with `python3 -B RPF_извлечённое/probes/rpf9_series_compare.py`. To select the 55 anchors in `rpf9_series_shared.tsv`: `unique_per_present_archive=1`, `distinct_relative_offsets>1`, `distinct_stored_hashes=1`. The native TOC has not been decoded.

## 15. Independent candidates for original names

In three examined archives (`core_licenses_data.rpf`, `core_restofworld001.rpf`, `story000_bvh4.rpf`), eight pairs of “likely original name → physical XML offset” were obtained without using the native index. In each archive, one `SSetupData/datFile` points to `content.xml`; the only XML with the root `CDataFileMgr__ContentDef` was matched to that name. Five `loadPath` references from these XML files were matched to `CDataFileMgr__ContentChangeSetDef` using the naming rule `ccs_<id, case-insensitive>.meta`. These names are inferred from references, not read from RPF metadata.

No matches were found in the initial regions or candidate tail regions of these three archives for any name in the tested forms: UTF-8, UTF-16LE, and 32-bit JOAAT with an initial state of zero (both byte orders). Paths and basenames, case variants, and separators were tested. The table `RPF_извлечённое/rpf9_xml_name_anchors.tsv` contains offsets, reference sources, and XML SHA-256 hashes. Reproduce with: `python3 -B RPF_извлечённое/probes/rpf9_name_anchors.py`.

## 16. Control example: RPF address and PSIN node references

In `script.rpf`, the embedded archive starts at `0xC4000`. The range `[0x10A000,0x10A1E2)` in the outer file (relative start `0x46000` within the embedded archive) is one complete zlib stream. It produces 992 bytes of PSIN. This is physical record number 29, zero-based, in the recovered list; it is not a native index number.

In the decompressed buffer, section `0x01393259` occupies `[0xC0,0xE0)` and contains two 16-byte records. For node 0, the `first_child` field at `0xC6` is `01 00` (`u16le 1`); for node 1, the `parent` field at `0xD4` is `00 00` (`u16le 0`). A record's address within PSIN is `0xC0 + 16*index`. The value `0xFFFF` denotes a missing reference. Node number, node ID, and source zlib stream address belong to different levels: references between nodes do not store physical RPF addresses. Finding a resource by name or archive identifier requires a separate mapping from `RPF record → packed byte range`.

## 17. Searching for bit-packed pointers to known PSIN streams

Four embedded archives with independently recovered PSIN addresses were checked (`script+0xC4000`, `data+0x9EF3000`, `data+0x690000`, `levels+0x1C000`) for bit fields without requiring byte alignment: LE/BE, every bit phase, absolute and base-relative addresses, and units of bytes or 16/512/4096. For each address set, the minimum width (at least 12 bits), the next width, and 24/32 bits where values fit were checked. Each configuration was checked separately for unordered matches and chains with a fixed bit stride of 8–1024 in both forward and reverse physical order. Synthetic records with a 19-bit address at bit phase 5 and stride 22 are detected in both bit orders.

Across 984 configurations, the longest fixed-stride chain contains two addresses. In some narrow-field tests, all sought values were found, but the complete match set also included at least one random control in every such test; without a positional rule, these are not pointers. The result constrains the tested direct bit-packing models but does not rule out arbitrary record ordering or other widths. See `RPF_извлечённое/rpf9_bit_pointer_trials.tsv`; reproduce with: `python3 -B RPF_извлечённое/probes/rpf9_bit_pointers.py`.

Separately, we checked whether absolute addresses had been replaced by differences between neighboring starts or gaps after the previous object. Sequences of six values were searched across all 14 embedded archives with a verified PSIN list, in forward and reverse physical order. ULEB128, big-endian VLQ, and integers 1–4 bytes wide in both byte orders were checked, using units of 1, 16, 512, and 4096. Uniform sequences and patterns shorter than eight bytes were excluded. None of the 837,808 windows matched; the synthetic positive control passed. This check does not rule out records with interleaved fields or nonphysical ordering. See `RPF_извлечённое/rpf9_pointer_delta_trials.tsv`; reproduce with: `python3 -B RPF_извлечённое/probes/rpf9_pointer_deltas.py`.

## 18. Contract for a future unpacker

* Keep separate stages: verify `.torrent` ranges → bounded reads of RPF candidates → zlib stream decoding → resource validation. Do not execute anything from the archive or modify source files.
* Name the first implementation `salvage`. For every result, preserve the archive chain with bases, the absolute stream range or list of ranges, computed decompressed size, resource kind, status of each check, offset-derived name, SHA-256, and raw bytes. Do not label offset-derived names as original. If an embedded archive's size is unknown, store `null`; do not substitute the end of the root file.
* Limit decompression volume, section count, and nesting depth; check overflows, sizes before reads, cycles, and duplicate references. XML must not load external entities. Do not use untrusted metadata paths to write outside the selected directory.
* Regression checks: the three exact source-file SHA-256 values and specified chunks; 259 XML, 1012 PSIN/1012 CHKS, 20,379 nodes, two UIHM maps, one HMAP, 858 SPS1, and an index of 30,683 offsets. Negative tests: truncate a stream or section, corrupt Adler-32/CHKS, provide 64 KiB of valid output at the boundary between two separate resources, and remove a neighboring file from a torrent-piece region. For a future `list` mode, compare every entry against an actually confirmed range, including cases without zlib.
* Before full `list/extract`: establish the index structure and reading method, original names/hashes, exact boundaries and roles of tail regions, and records without zlib. Before packing, separately establish serialization rules and pass a read-back check. Within resources, the `LSNS` payload, semantics of individual PSIN fields, and exact HMAP height encoding remain unknown.

## 19. Movie header and boundary verification

- The root mode-1 header gives `D=0x150`, `B=0x56A0`, `A=0x6AC0`, `C=0x49F0`, and `N=515`. Walking the movie lengths found exactly 515 entries in physical order, with 505 `KB2n` and 10 `KB2j`. The region from `A` to `EOF−C=0x20E74B000` consists entirely of those entries and zero padding. The first starts at `0x7000`, and the last ends at `0x20E74A0D4` (exclusive). Every start is 4 KiB aligned. The gaps after `0x17E76FFC` and `0x3F486FF8` each include one extra zero page.
- In all 515 headers, the entry length is `u32le(+0x04)+8`. Frame count `F` is repeated at `+0x08` and `+0x10`. At `0x30+12*u32le(+0x28)` there are `F+1` little-endian 32-bit words. These give the relative frame starts with a flag in bit 0, followed by a terminal word equal to the entry length. After masking bit 0, the first start lands at the end of the index. All 425,440 intervals increase and stay within the largest frame size at `+0x0C`. Audio packet length fields fit inside those intervals and leave room for a video packet.
- Of 510 prior offset-named files, 411 are longer than their corresponding ranges, 85 shorter, and two match by length. The other 12 starts are absent from the verified list. The video header supplies the exact length for this archive. The count confirms the meaning of `N` in this sample. A search for known addresses and sizes as literal LE/BE 32-bit words found zero matches in `[0x40,D)`, `[D,B)`, `[B,A)`, or the trailer. Start addresses were also checked in 16-byte and 4 KiB units and relative to the first movie in bytes and pages. Addresses beyond the 32-bit range were excluded from the literal byte-address test. The native TOC and names remain unknown.
- Across the 193-header corpus, the 16-byte block at `+0x60` has 192 distinct values. At each 16-byte position from `+0x70` through `+0x140`, all 193 blocks differ. This bounds the recurring prefix described in section 8. The transformation remains unknown.
- With exact ranges, mobipeg decoded every video frame in 13 of 14 samples, covering both Bink2 revisions and 1280×720 examples. The portrait `KB2n` at `0x3D04D000` (720×1280) returned `Invalid data found when processing input` before producing a frame. Audio decoding and full video decoding of all 515 entries were outside this check. [mobipeg, commit a7d1dd4](https://github.com/quatric/mobipeg/tree/a7d1dd4fef62ba3b6979ade21c4947b5e8e7d201)
- Pillow rejected all 74 prior JPEG candidates (73 from `textures.rpf` and one from `common.rpf`). None of the carved ranges ends with `FF D9`. A valid JPEG header remains unconfirmed for these ranges.

From the archive root:

```sh
python3 -B RPF_извлечённое/probes/rpf9_movie_layout.py movies.rpf > /tmp/rpf9-movie-layout.tsv
```

The probe reads local bytes and reports half-open ranges. Torrent-piece SHA-1 hashes were outside this run.

## 20. Mid-archive ADAT audio bank

The `ADAT` bank at `0x351EEE000` starts 49.96% into the archive and contains 973 entries. For the two tested tracks, tag `0x76` gives the sample count at `+0x00`, the 48,000 Hz rate at `+0x08`, and codec `0x0B` at `+0x1C`. Tag `0x55` gives the payload range. The payload uses `0x800`-byte blocks containing Opus packets with `u16le` byte lengths and zero padding at the end of the block. Block starts matched the sample indices in tag `0xA3`.

Track 319 occupies `[0x352BEC20C,0x352BF020C)` and yields 2.000 s. Track 479 occupies `[0x3531E3738,0x353204F38)` and yields 16.000 s. libopus decoded every packet of both tracks. The WAVs retain the first 96,000 and 768,000 samples, respectively, using the declared lengths. The role of the extra 1,920 samples and the exact start alignment remain unverified.

Direct Ogg Opus extraction from the archive root, with no WAV intermediate or re-encoding.

```sh
python3 -B RPF_извлечённое/probes/rpf9_extract_opus.py audio.rpf --bank 0x351EEE000 --track 479 --out track-479.opus
```

### Size estimate

The 28,543,454,672-byte file contains 7,097 candidate `ADAT` banks with consistent table bounds. They contain 1,604,709 entries, and the sizes of the readable `0x55` data tags sum to 15,163,704,233 bytes. The sum includes block padding, counts duplicates, and excludes data outside the detected banks. In samples from the two largest groups, estimated Ogg output was about 74% of stored payload at 32 kHz and 92% at 48 kHz. The detected material would roughly occupy 13–15 GB in compressed form before filesystem overhead. The current tool checks one track per invocation and accepts the observed 48 kHz mono Opus variant.

### Sample rates and codecs

The counts in this subsection come from an early scan. The updated `0x48` results appear below.

Individual track headers with tag `0x76` give these rates for codec `0x0B` (Opus): 11,025 Hz (2 mono), 12,000 Hz (1 mono), 22,050 Hz (4 mono), 28,000 Hz (8 mono), 32,000 Hz (1,066,527 mono), and 48,000 Hz (448,583 mono, 4,583 stereo). Stream tags `0x48` give six additional two-channel entries and one four-channel entry, all Opus at 48,000 Hz. Codec `0x05` (XMA2 in vgmstream) appears on 355 mono tracks at 32,000 Hz. Five mono 48,000 Hz tracks use unidentified codec `0x02`. These are archive header values. Full decoding across all configurations remains unverified. [vgmstream AWC parser](https://github.com/vgmstream/vgmstream/blob/24ffd6ce3e1f34fdebf9f5316d18ad2e162838ce/src/meta/awc.c#L97)

### Codec ID 0x02 payloads

Five entries with codec ID `0x02` contain eight distinct `0x55` ranges. Before the `E1 AC FF FF` marker, each range contains 8192-byte blocks that read as 1024 complex numbers made of little-endian float32 pairs. An inverse FFT yields 2048-sample windows with a 1024-sample hop. Track 7 of bank `0x69889A000` has a relative RMS difference of 0.057% between its overlapping halves. The test WAV uses the 22,933 samples from tag `0x76` at `+0x0C`. That field remains unverified for some repeated payloads. The test assumes a zero Nyquist coefficient.

### Batch extraction

The script scans for readable `ADAT` banks and exports every readable `0x55` range, including repeated tags. Compatible Opus and XMA2 packets are wrapped without re-encoding. Other ranges are copied verbatim to `.bin` files. The `manifest.tsv` records the codec, rate, channels, source ranges, and status. Use `--raw` to preserve source payload bytes for every track. Discovery uses ADAT headers.

Counting each tag separately gives 1,531,938 ranges totaling 15,357,741,993 bytes.

```sh
python3 -B RPF_извлечённое/probes/rpf9_extract_audio.py audio.rpf --out-dir audio-extracted
```

The experimental Linux C port supports the same `--bank` and `--raw` options. It uses libopus to check packet durations when the library is available.

```sh
cc -O3 -std=c17 RPF_извлечённое/probes/rpf9_extract_audio.c -ldl -o rpf9-extract-audio
./rpf9-extract-audio audio.rpf --out-dir audio-extracted-c
```

### Opus streams with tag 0x48

Tag `0x48` starts with three little-endian 32-bit fields for block count, block stride, and stream count, followed by 16-byte stream descriptions. Each `0x55` block starts with a 24-byte record per stream. Fields at `+4`, `+8`, and `+12` give the group count, overlap, and logical block length in samples. One little-endian 32-bit absolute seek value per group follows the records. After alignment to `0x800`, groups of `0x800` bytes appear in stream order. Each group contains packets with little-endian 16-bit lengths and zero padding.

Seek positions advance by 960 per mono packet and 1,920 per stereo packet because the table counts individual PCM values. The first seek plus the overlap equals the preceding block’s logical end. The physical end can fall before or after that marker. Repeated packets at the same position match byte-for-byte. A scan of `audio.rpf` found 50 banks with multiple `0x48` streams. FFmpeg decoded all 50 exported files.

## 21. Follow-up on HEADER_NOTES.md

A fresh scan of `levels.rpf` and `pack/story000/story000.rpf` yielded 2,493 header candidates (2,051 and 442). The filter requires `9FPR`, 16-byte alignment, plausible bounds, zeroes at `+0x34..+0x3F`, and the known prefix at `+0x40`. The last two checks are selection criteria. Torrent-piece hashes were outside this run.

- All 1,162 mode-1 headers match `A−B=align16(N*(8+v))`. The highest observed `N` for `v=1` is 253. The lowest for `v=2` is 257. This sample contains no counts from 254 through 256. Of 1,071 blocks with bytes past `N*(8+v)`, 1,070 have nonzero bytes there. The block size is verified. The layout of individual records remains unknown.
- Across all 2,493 candidates, `u32le(+0x18)=0x47A00` occurs exactly when `u32le(+0x30)=0x10` (804 cases). `D−u32le(+0x30)=0x140` holds for 2,428 candidates. All 65 exceptions have `u32le(+0x18)=0x67A00` and modes 6–8. The difference varies in that family.
- For mode 2 with `u32le(+0x30)=0x10`, the earlier formula `align16(N*(8+2*v)+8)` matches 1 of 95 cases in `levels.rpf` and 16 of 22 in `story000.rpf`. A different local pattern appears in `levels.rpf`. For 51 mode-2 headers, `A−B=C`, and each has a mode-3 candidate immediately after it in physical order.
- In `levels.rpf`, 90 of 92 mode-3 headers have a mode-2 predecessor. For 56 pairs, the region from the predecessor's `A` boundary to the next `9FPR` closes with complete zlib streams, zero padding, and a nonempty `C`-byte trailer candidate. In 42 of those pairs, `A−B=C`. The bytes in the two equal-length regions differ in every case. For example, `levels.rpf+0x11C10D000` has five streams, trailer range `[0x11C124300,0x11C124340)`, and the next header at `0x11C125000`. The trailer's role and the relationship between mode-2 and mode-3 entries remain unresolved.

```sh
python3 -B RPF_извлечённое/probes/rpf9_header_geometry.py levels.rpf pack/story000/story000.rpf
```

## 22. Additional size formulas

- All 90 mode-2 candidates before mode 3 in `levels.rpf` match `A−B=align16(N*(8+2*v)+32)`. All 90 have `v=1`, `u16le(+0x20)=6`, `u32le(+0x18)=0x47A00`, and `u32le(+0x30)=0x10`. Adding 31 gives the same results after alignment, so the size of any extra field remains undetermined. Among the other 968 mode-2 headers in the expanded sample, just 16 fit this equation.
- All 68 mode-4 headers match `A−B=align16(8*N)+align16(212*N+8)`, with observed `N=1..23` and `v=1`. The equation was derived from 66 headers in `levels.rpf` and checked on two headers in `story000.rpf` and `shared_minigame000.rpf`. All have `u16le(+0x20)=7`, `u32le(+0x18)=0x57A00`, and `u32le(+0x30)=0x180`. The two aligned terms describe the observed length. A physical boundary between sub-blocks remains unverified.
- The expanded sample contains 4,373 aligned candidates in 22 files under the same filter. All 2,908 mode-1 candidates match the formula in section 8. The scan adds `N=254, v=1` at `audio.rpf+0x2806A6000`. The smallest observed `N` for `v=2` remains 257. The sample contains no `N=255` or `N=256` cases. Fields `+0x10` and `+0x14` each have 4,373 distinct values.

## 23. N and 0DRP object starts

Between successive candidates, the probe walked complete zlib streams after `A` with zero padding to 16-byte alignment. It counted a matching boundary when a nonempty `C`-byte trailer candidate followed those streams and only zero padding remained before the next header. An object start requires eight zero bytes and `0DRP` at `+0x10` in the decoded stream.

| Mode | Candidates | Boundary matches | Start count equals N |
|---|---:|---:|---:|
| 2 before 3 | 90 | 56 | 56 |
| 4 | 68 | 49 | 49 |
| 5 | 52 | 50 | 50 |
| 6 | 80 | 72 | 70 |

All 5,847 `0DRP` starts have `u32le(+0x14)=0x7A`. Other object types account for the two mode-6 mismatches. `levels.rpf+0x460561000` has 24 `0DRP` objects and one PSIN with a verified CHKS. `levels.rpf+0xEC2771000` has 48 `0DRP` objects and 11 headers with `u32le(+0x10)=0x106` and a pointer in the `0x50000000` address space at `+0x08`. Ten pointers land in their decoded stream. The entry at `0xEC2781010` points to `0x500104C0` in the continuation after its first 65,536-byte stream. Counting these types gives `N` in all 227 samples with matching boundaries.

In 49 mode-4 archives, 299 recovered ranges were used to look for adjacent LE/BE `u32` offset and size words in either order. Relative starts were tested in bytes and in units of 16, 512, and 4096, with stored and decoded lengths. No pair matched in `[B,A)`, including its first and last `align16(8*N)` bytes.

## 24. Mode-3 object boundaries

In `levels.rpf`, 91 of 92 mode-3 archives close at the next header using complete zlib streams, a `C`-byte trailer candidate, and zero padding. In 55 cases, individual streams required the higher limits of 8 MiB input and 32 MiB output. All 91 object counts match `N` (29,749 objects): 2,073 `0DRP` starts, 2,498 headers with `0x106` at `+0x10`, and 25,178 pointer-shaped headers. All 200 torrent pieces covering the inspected ranges of the 92 mode-3 archives matched their SHA-1 digests, including the exception below.

For the last group, the test checks eight zero bytes before the aligned pointer at `+0x08`, an address in `0x50000000..0x5FFFFFFF`, and 16 zero bytes immediately before the stored zlib stream. It also requires a second pointer at `+0x10`, an address within the first decoded stream, or a 65,536-byte first fragment. The example at `0x1384FC010` has fragments of 65,536 and 45,632 bytes. Its pointer at `+0x08` resolves to relative address `0x1B1F0` in the second fragment.

The exception at `levels.rpf+0x1433C0000` contains a 43-byte span ending at the last nonzero byte after the zlib stream ending at `0x146EE62B5`. The span begins at `0x146EE62C0`. Zero padding follows up to the next zlib stream at `0x146EE7010`. Manually resuming there accounts for 683 object starts (`N=683`), trailer range `[0x1472FF000,0x147328710)`, and the next header at `0x147329000`. The short block remains unidentified. The probe stops there.

## 25. Additional header-region checks

- Across all 4,373 candidates, `D−u32le(+0x30) != 0x140` holds exactly for `u32le(+0x18)=0x67A00` (67 cases). For that group, both possible orders of the `u32le(+0x30)`-byte part and the remaining bytes after `0x140` were tested. None of the 335 bounded inputs fully decoded with raw DEFLATE, zlib, gzip, bzip2, XZ, Zstandard, or raw LZ4. All seven positive controls passed. The purpose of the parts remains unknown.
- Four 16-byte values at `+0x50` separate by fixed header fields throughout the expanded sample. The `9B7BF4BD` prefix occurs with `u32le(+0x18)=0x67A00`; `CC0FC398` occurs with `0x77A00`. The `FB9F7567` prefix occurs with `0x47A00` and with `0x57A00` when mode is at least 3 and `u32le(+0x30)=0x180`. The remaining samples have `B8CF2622`.
- The first nonzero block after `A` is at `base+align4096(A)+16` in all 92 mode-3, 68 mode-4, and 52 mode-5 candidates. This also holds for the 90 mode-2 predecessors from section 22. Other families have different offsets.
- For 2,783 object starts across 36 mode-3 archives, literal `u32` addresses were searched in `[D,B)`, `[B,A)`, and the trailer, including absolute and relative page numbers and page-rounded addresses. No literal word matched. XOR with `+0x14` gave three isolated hits, with no adjacent word matching the known length.

Before modes 7 and 8 are included, `RPF_извлечённое/rpf9_object_starts.tsv` gives 35,608 object starts in 318 recovered archives. Its ranges cover only the first stream, with a physical ordinal. All 164 torrent pieces covering the 227 mode-2, mode-4, mode-5, and mode-6 archives in the table also matched SHA-1.

```sh
python3 -B RPF_извлечённое/probes/rpf9_header_geometry.py levels.rpf pack/story000/story000.rpf pack/shared_minigame000/shared_minigame000.rpf --starts-tsv /tmp/rpf9-object-starts.tsv
```

## 26. Trailer alignment and digest checks

- The exact end of the last zlib stream was rechecked for all 318 archives in the object-start table. Each trailer starts at that end rounded up to 16 bytes or to a 4 KiB page relative to the archive base. The rule “4 KiB for `C>=0x800`, otherwise 16 bytes” misses 35 cases. Of the 91 automatic mode-3 matches, two trailers use 16-byte alignment (`C=0xE0` and `0x130`); 89 use page alignment.
- In 315 cases the next header is on the first 4 KiB boundary after the trailer. Two mode-3 cases have an additional zero page. The header after `levels.rpf+0x73BB1D000` (mode 5) is earlier than the next 4 KiB boundary. The `C` field alone does not determine both alignments.
- Across 22 root headers, 13,464 comparisons tested fields `+0x10`, `+0x14`, and `+0x28` against CRC-32, CRC-32C, Adler-32, xxHash32 with a zero seed, and two JOAAT variants, including the signed-byte PSIN variant seeded with `0x3FAC7125`. Tests covered both byte orders, header regions, the trailer, and variants with header fields zeroed. No tested digest matched.

## 27. Resynchronization in modes 7 and 8

Modes 7 and 8 contain gaps with nonzero bytes between zlib streams. Searching for the next aligned zlib header and validating the complete stream gives exactly `N` recognized starts in all 83 mode-7 and 32 mode-8 archives, totaling 167,113 objects. Recognition uses the `0DRP` and `0x106` shapes above and verified PSIN checksums. All 1,326 PSIN starts in this scan had valid CHKS. All 442 torrent pieces covering these 115 ranges through the next header matched SHA-1.

For 71 mode-7 and all 32 mode-8 archives, a `C`-byte trailer candidate follows the last zlib stream, with only zero padding up to the next header. Twelve mode-7 archives contain additional nonzero bytes after the last zlib stream. Starting at the next header, rounding the position after the last nonzero byte up to 16 and subtracting `C` gives a page-aligned trailer candidate in all 12. The same backward calculation reproduced the known trailer starts in the 103 control archives. The 12 remain estimates because the intervening bytes are unparsed.

Including modes 7 and 8, before adding the extended mode-2 cases, gives 187,404 object starts across 421 archives. The table includes cases where the count matches `N` and the trailer boundary was found after zlib.

```sh
python3 -B RPF_извлечённое/probes/rpf9_header_geometry.py levels.rpf pack/story000/story000.rpf pack/shared_minigame000/shared_minigame000.rpf --resync-high --starts-tsv /tmp/rpf9-object-starts-with-high.tsv
```

## 28. Extended mode-2 samples

For mode 2 with `u32le(+0x18)` equal to `0x57A00` or `0x87A00`, 330 ranges up to 32 MiB were tested through the next header. Resynchronizing at aligned zlib streams gives exactly `N` starts in all 330 cases (40,919 objects). In 310 cases the `C`-byte trailer candidate follows the last zlib and the gap to the next header is zero. All 545 torrent pieces covering the 330 ranges matched SHA-1.

A separate sample of 79 mode-2 archives in `anim.rpf` yields 8,647 starts, matching `N` in every archive. For 260 additional starts at page offset 16, the first decoded stream length is a multiple of 65,536. Each pointer at `+0x08` resolves into a continuation when joining streams up to the next recognized start. This check applies to the sampled archives.

The 4,373 headers contain 10 distinct `+0x28` values. No match was found against 71,865 local text tokens and variants using JOAAT, PSIN JOAAT, CRC-32, FNV-1a, or xxHash32 in either byte order.

The current table contains 225,925 object starts across 731 archives, including 38,521 starts from the 310 extended mode-2 archives.

```sh
python3 -B RPF_извлечённое/probes/rpf9_header_geometry.py levels.rpf pack/story000/story000.rpf pack/shared_minigame000/shared_minigame000.rpf --resync-high --resync-extended-mode2 --starts-tsv /tmp/rpf9-object-starts-full.tsv
```

## 29. Mixed PSIN fragments

All 734 PSIN records at `pack/story000/story000.rpf+0x11B0E5000` pass CHKS. zlib streams suffice for 708 records. The other 26 also require stored raw bytes. The archive has 1,164 complete zlib streams and 96 raw fragments totaling 4,780,504 bytes. Raw fragments begin after aligning the preceding stream end to 16 bytes. Verification uses complete 65,536-byte raw blocks between zlib streams and a final partial block bounded by the size and CHKS. The corresponding torrent pieces matched SHA-1.

- The PSIN at `0x11B4A0000` has 71 raw blocks in `[0x11B550770,0x11B9C0770)` between zlib streams. Adding those 4,653,056 bytes gives the declared 5,521,528 bytes and a valid CHKS.
- For the PSIN at `0x11BD00000`, zlib supplies the first 12 bytes of CHKS. The final eight, including the digest and `zppp`, are stored raw at `0x11BD07C10`. Checking only for a marker that starts in the raw bytes misses this case.

```sh
python3 -B RPF_извлечённое/probes/rpf9_psin_mixed_probe.py pack/story000/story000.rpf --base 0x11B0E5000 --trailer-start 0x11C146000
```

## 30. What RPF7 and RPF8 suggest

- RPF7 uses 16-byte entries and a separate name table in the front metadata. A directory names a contiguous range of child entries by start index and count. The directory marker `0x7FFFFF00` did not occur in either byte order across 166,094,704 front metadata bytes from 4,373 RPF9 candidates. This tests the raw representation. [CodeWalker RPF7 reader](https://github.com/dexyfex/CodeWalker/blob/master/CodeWalker.Core/GameFiles/RpfFile.cs#L2007)
- Swage's RPF8 reader uses 24-byte entries containing a name hash, extension ID, offset, and sizes. Its source includes a disabled branch that reads names from EOF. In RPF9, the `C`-byte trailer is a candidate for name or identity metadata because the 13 archives in section 11 share identical trailers while their physical data layouts vary. [Swage RPF8 reader](https://github.com/0x1F9F1/Swage/blob/master/src/games/rage/rpf8.cpp#L677)
- In all 2,908 mode-1 candidates, `[D,B)` has room for at least `24*N` bytes (observed minimum 35.7 bytes per entry). The compact `[B,A)` block has roughly `9*N` or `10*N` bytes. This motivates testing separate lookup and file-range tables. The purpose of each region remains unverified.
- The front metadata and trailer of `movies.rpf` gave zero matches at any byte offset for the packed offset and size of any of the 515 verified movies in the RPF7 or RPF8 layouts. RPF8 tests used 16-byte units, rounded sizes, and a 16-byte resource-prefix variant. RPF7 tests used 512-byte units and both size-field variants. The starts of 116 movies exceed the reach of a 24-bit RPF7 sector number. Synthetic controls passed for both layouts.

## 31. Testing address fields in 13 nested archives

The zlib streams shared by the archives in §14 retain their physical order. Across 78 archive pairs, 1,445 shared-stream order comparisons gave zero inversions. The pair at `0x7200000` and `0x9326000` shares 41 streams. The ordinal shifts by one for most of them and by two for the last moved stream. These are scan ordinals. Their relationship to RPF9 entry numbers is still unknown.

We tested 2-, 3-, 4-, 5-, and 8-byte fields in `[D,B)` and `[B,A)` against nine moved streams present in at least four archives each and the first large stream in all 13. Candidate slots were aligned to a region boundary or derived from forward or reverse scan ordinals. We fitted each candidate on two archives with different target values and tested it on the remaining archives. Models covered addresses, lengths, adjacent-address deltas, byte units of 1, 16, 512, and 4096, and constant XOR or modular addition, with optional corrections derived from header fields `+0x10` and `+0x14`.

None of the 6,480 two-archive matches predicted the values in every held-out archive. Synthetic controls passed, including a slot that moved with the scan ordinal. The native index layout remains unresolved.

Evidence: `RPF_извлечённое/rpf9_series_slot_trials.tsv` and `RPF_извлечённое/rpf9_series_slot_matches.tsv`. Reproduce with NumPy installed.

```sh
python3 -B RPF_извлечённое/probes/rpf9_series_slot_trials.py
```

## 32. Physical payload order and nesting

A sequential walk from `A` to the trailer start in the 13 archives from §11 found 14,654 complete zlib streams with zero-filled gaps. Input and output caps of 8 MiB and 32 MiB recovered 26 streams missed by the earlier probe, two per archive. Each has a version `0x7A` `0DRP` resource header.

Each archive contains exactly `N = 1115` recognized starts in physical order, comprising one PSIN with a verified CHKS and 1,114 `0DRP` resources. A total of 159 resources span two streams. In three cases, the pointer at `+0x08` resolves to the second stream of the same resource. The other checked pointers resolve to the first stream. A total of 10,872 object starts occur 16 bytes past a page boundary. All continuation streams start at other page offsets, as do many object starts.

In `script.rpf`, the nested ranges `[0x1000,0xC3B90)` and `[0xC4000,0x4282B0)` match the saved child archives byte for byte and precede the outer archive’s candidate trailer. These are nested containers. The checked `data.rpf` copies store their resources in sequential runs. The structure of the native index and directory relationships remain unknown.

Evidence: `RPF_извлечённое/rpf9_payload_runs.tsv`.

```sh
python3 -B RPF_извлечённое/probes/rpf9_payload_runs.py
```

## 33. Raw payloads between streams

We checked 3,777 nonzero gaps between complete zlib streams in 49 mode-7 and mode-8 children of `levels.rpf`. These children have verified object starts and a trailer within 24 MiB of the archive header. The candidate raw start is `align16(zlib_end)`. A model covers the gap when its proposed extent contains every nonzero byte and ends before or at the following zlib stream.

- Descriptors in the preceding resource account for 3,725 gaps. The observed descriptor shape is `u32le tag, u32le size, u64le pointer`, with tags `0xEA6B3A96` and `0xED86F522`. Pointers expressed as `0x60000000 + offset` identify ranges of raw graphics data. For resource `0x86C028010`, the descriptors span 262,272 bytes. The second zlib stream supplies 262,144, and the final 128 bytes are stored raw at `0x86C05DC20`. CodeWalker uses a comparable system/graphics address distinction for RAGE resources. [ResourceData.cs](https://github.com/dexyfex/CodeWalker/blob/master/CodeWalker.Core/GameFiles/Resources/ResourceData.cs)
- A further 38 gaps match a table reached by the pointer at `0DRP+0x40`, immediately after the decoded bytes. A little-endian 32-bit count precedes tag/length pairs and their payloads. Six other gaps match a count followed by 16-byte records, reached through `0DRP+0x50`. Resource `0x644288010` has both tables in sequence. Its next zlib stream decodes to the extent declared by the graphics descriptors.
- The seven counted tables contain 5,789 records, including 5,063 distinct records. One pair of resources shares 406 records in different relative order. No record matched an aligned 16-byte block in its owning archive’s `[D,A)` header regions. The record meaning is unknown. Six gaps have nonzero suffixes beyond the matched structures.

Evidence: `RPF_извлечённое/rpf9_gap_payloads.tsv`. Reproduce with NumPy installed.

```sh
python3 -B RPF_извлечённое/probes/rpf9_gap_payloads.py
```

## 34. Walk across all 22 archives

The pass covered all 22 local `.rpf` files, totaling 161,307,137,472 bytes. It found 4,373 candidate headers. For 20 archives, the walker checked aligned zlib streams from `A` to the next detected header or the outer trailer candidate. It skipped two regions with overlapping geometry. Across the remaining 4,267 regions it found 2,240,509 complete, nonempty zlib streams. Recognized starts matched `N` in 3,747 regions. In 3,654 of those the candidate trailer also fit, including 2,941 with only zero-filled inter-stream gaps.

- `levels.rpf` yielded 1,117,824 zlib streams and 1,859 regions with both the count and boundary match. A separate check explained 16,801 of 16,850 nonzero gaps across 97 bounded mode-7/8 children. The 26 tables of 16-byte records contain 12,920 distinct records out of 29,621. None matched an aligned block in its owning archive’s front metadata.
- The full `core_bvh4.rpf` pass found 1,282 candidate headers. In 1,260 regions the count and candidate trailer fit with zero-filled gaps. The earlier spot-check inventory listed 29 candidates for that archive.
- `story000_bvh4.rpf` contains four standalone XML files, matching `N=4`. They and the zero padding fill the area up to the candidate trailer. `appdata0.rpf` has `N=2` and yielded no matching zlib streams. `shaders.rpf` has two aligned `SGD5` signatures at `0x9000` and `0x10287000`. Its three outer entries still lack confirmed ranges.
- The Bink length walk accounts for all 515 movies up to the candidate trailer in `movies.rpf`. The ADAT walk in `audio.rpf` confirmed 7,160 banks, 1,606,241 stream entries, and 1,603,123 nonempty `0x55` payload tags. Their lengths sum to 22,687,824,934 bytes and can include shared ranges. These counts describe the inner audio structures. The outer audio index remains unresolved.

The counts describe the available local bytes and the tested `0x78` zlib profile. Stream validation and torrent-piece verification measure different things.

Results: `RPF_извлечённое/rpf9_payload_coverage.tsv` and `RPF_извлечённое/rpf9_levels_gap_coverage.tsv`. Reproduce the main walk.

```sh
rg --files --hidden --no-ignore -g '*.rpf' -g '*.RPF' -g '!levels.rpf' -g '!RPF_извлечённое/**' -0 \
  | xargs -0 python3 -B RPF_извлечённое/probes/rpf9_walk_all.py --native-media --classify-raw
python3 -B RPF_извлечённое/probes/rpf9_walk_all.py levels.rpf
python3 -B RPF_извлечённое/probes/rpf9_gap_payloads.py --max-span-mib 256 \
  --out RPF_извлечённое/rpf9_levels_gap_coverage.tsv
```

## 35. Cross-check of the second agent's progress

The second agent's movies walk independently confirmed the 515 Bink ranges in §19, from `0x7000` through `0x20E74A0D4` (exclusive). The padding up to the candidate trailer at `0x20E74B000` is zero-filled. Its report and probe are in `RPF_извлечённое/evidence/movies_walk.tsv` and `RPF_извлечённое/probes/movies_walk.py`.

### The 13-child series in data.rpf

The nested-table probe parsed each child header at its correct base, then sliced the outer file using `data[B:A]` and `data[D:B]`. Those offsets are relative to the child. Using `data[base+B:base+A]` and `data[base+D:base+B]` yields **13 distinct SHA-256 hashes in each region**. Repeating the offset-omitting slices reproduces the two reported groups of five identical blocks. The nested row dumps and any tests that consume those rows inherit the slicing error.

The trailer discrepancy came from treating the last sampled child as the last child in the file. The child at `0x9326000` has a candidate trailer at `[0x9E0B000,0x9E124A0)`. The next `9FPR` is at `0x9E13000`, after `0xB60` zero bytes. The different marker cited in the second report came from the outer EOF at `0x1CB3C870`. All 13 child trailers match the SHA-256 in §11. The recheck also confirmed that the first stream of child `0x11DE000` inflates to 7,176 bytes and starts with a PSIN header.

### What the table tests establish

The mode-1 length rule remains `A−B = align16(N*(8+v))`, with `v = u32le(+0x2C) & 0xFF` in the tested corpus. In `movies.rpf`, 515 hypothesized 10-byte rows account for 5,150 of the 5,152 bytes in `[B,A)`. In the 13 mode-2 `data.rpf` children, `A−B=0x3450` exceeds `12*N` by 12 bytes. The `script.rpf` child at `0xC4000` has `A−B=0x2F80`, 16 bytes beyond `12*N`. Both examples fit `align16(12*N+8)`. Those sizes leave the actual row layout and the placement of the extra bytes open.

| Direct check | Result |
|---|---|
| All 515 movies against `[0x40,A)` in `movies.rpf` | No literal 32-bit LE or BE match for starts, ends, lengths, start page numbers, frame counts, widths, or heights. |
| 1,012 observed PSINs against the base-relative `[B,A)` in the second `script.rpf` child | No literal 32-bit LE or BE match for stored starts or ends, in absolute or child-relative bytes. |
| Recorded zlib members against each corrected `[B,A)` in the 13 `data.rpf` children | No literal 32-bit LE or BE match for absolute or relative offsets, stored lengths, or decoded lengths. |
| XOR of corrected `[B,A)` with `[D,B)` at phases 0–3 | No movie or PSIN range match. Phase 2 produced two isolated series hits: a stored length of `0x30DB` at offset `0x1120D` in the child at `0x31BB000`, and an absolute start of `0x6634010` at offset `0x1169F` in the child at `0x5CAC000`. The hits provide no matching offset-and-length pair or repeated layout. |

The second agent also reported negative movie-table trials for constant XOR or addition in presumed row order, repeating header bytes, SHA-256-derived masks, candidate AES-128 and AES-256 configurations, and selected MD5, SHA-1, and JOAAT fingerprints. These are limited tests of the chosen inputs and models. The actual encoding and the roles of `+0x10` and `+0x14` remain unknown.

Audit trail: `RPF_извлечённое/PROGRESS_AGENT2.md` and its evidence preserve the original observations. Use `RPF_извлечённое/rpf9_data_series_verified.tsv` and the base-relative ranges above for nested comparisons.

## Sources

* [BEP 3: The BitTorrent Protocol Specification](https://www.bittorrent.org/beps/bep_0003.html) covers bencode, `info_hash`, file order, and SHA-1 piece hashes.
* [RFC 1950: ZLIB Compressed Data Format Specification](https://www.rfc-editor.org/rfc/rfc1950) covers the zlib header, `FDICT`, and Adler-32.

The offsets, resource fields, and test results come from the local samples listed above. Public implementations of earlier RPF versions served as comparisons. RPF9 rules require evidence from the local samples.
