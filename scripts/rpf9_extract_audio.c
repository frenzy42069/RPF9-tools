/* Experimental C port of rpf9_extract_audio.py for Linux. */
#define _POSIX_C_SOURCE 200809L

#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

enum { BLOCK = 0x800, PAGE_PACKETS = 25, MAX_TAGS = 7, MAX_STREAMS = 8,
       MAX_OPUS_PACKET = 1276, PACKET_SAMPLES = 960, CHANNEL_RECORD = 24,
       MAX_JOINED_PACKET = MAX_STREAMS * (MAX_OPUS_PACKET + 2) - 2 };
enum { FLAG_FIRST_TAGS = 0x00010000, FLAG_EXTRA_TABLE = 0x00200000,
       FLAG_PREFIX_WORD = 0x00400000 };

typedef struct { const uint8_t *data; size_t size; } Archive;
typedef struct { size_t base, table, tags; uint32_t flags, entries; } Bank;
typedef struct { unsigned kind; size_t start, end; } Tag;
typedef struct { unsigned codec, rate, channels; uint32_t samples; bool known; } Format;
typedef struct { const uint8_t *data; unsigned bytes, frames; } Packet;
typedef struct { const uint8_t *data; unsigned bytes; } MultiPacket;
typedef struct { MultiPacket *packets; size_t count, capacity; } MultiChannel;
typedef int (*OpusDuration)(const unsigned char *, int, int);
typedef struct {
    OpusDuration duration;
    Packet *packets;
    size_t capacity;
    uint32_t crc[256];
} Mux;
typedef struct { uint_fast64_t banks, opus, xma2, raw, empty, invalid, bytes; } Totals;

static void fail(const char *message)
{
    fprintf(stderr, "rpf9_extract_audio: %s\n", message);
    exit(2);
}

static void system_fail(const char *path)
{
    fprintf(stderr, "rpf9_extract_audio: %s: %s\n", path, strerror(errno));
    exit(2);
}

static uint32_t le(const uint8_t *bytes, unsigned count)
{
    uint32_t result = 0;
    for (unsigned i = 0; i < count; ++i)
        result |= (uint32_t)bytes[i] << (8 * i);
    return result;
}

static void put(uint8_t *out, size_t offset, uint64_t value, unsigned count)
{
    for (unsigned i = 0; i < count; ++i)
        out[offset + i] = (uint8_t)(value >> (8 * i));
}

static void write_bytes(FILE *out, const void *data, size_t bytes)
{
    if (fwrite(data, 1, bytes, out) != bytes)
        system_fail("writing output");
}

static void make_output(char *path)
{
    for (char *slash = path + 1; *slash; ++slash) {
        if (*slash != '/')
            continue;
        *slash = '\0';
        if (mkdir(path, 0777) != 0 && errno != EEXIST)
            system_fail(path);
        *slash = '/';
    }
    if (mkdir(path, 0777) != 0)
        system_fail(path);
}

static Tag read_tag(Archive a, size_t base, size_t at)
{
    const uint8_t *p = a.data + at;
    uint32_t lo = le(p, 4), hi = le(p + 4, 4);
    size_t start = base + (lo & 0xfffffff);
    size_t size = (lo >> 28) | ((hi & 0xffffff) << 4);
    return (Tag){p[7], start, start + size};
}

static bool bank_at(Archive a, size_t base, Bank *bank)
{
    if (base > a.size || a.size - base < 16)
        return false;
    const uint8_t *p = a.data + base;
    uint32_t flags = le(p + 4, 4), entries = le(p + 8, 4), header = le(p + 12, 4);
    if (memcmp(p, "ADAT", 4) || (flags & 0xff00ffff) != 0xff000001 ||
        entries == 0 || entries > 100000 || header > a.size - base)
        return false;
    size_t first_tags = 16 + ((flags & FLAG_PREFIX_WORD) ? 4 : 0);
    size_t table = first_tags + ((flags & FLAG_FIRST_TAGS) ? 2 * entries : 0);
    if (flags & FLAG_EXTRA_TABLE)
        table += 4 * (entries + 1);
    size_t tags = table + 4 * entries;
    if (tags > header)
        return false;
    size_t tag_count = 0;
    for (unsigned i = 0; i < entries; ++i) {
        if ((flags & FLAG_FIRST_TAGS) && le(p + first_tags + 2 * i, 2) != tag_count)
            return false;
        tag_count += p[table + 4 * i + 3] >> 5;
    }
    if (tags + 8 * tag_count != header)
        return false;
    for (size_t at = tags; at < header; at += 8) {
        Tag tag = read_tag(a, base, base + at);
        if ((tag.start != tag.end && tag.start < base + header) || tag.end > a.size)
            return false;
    }
    *bank = (Bank){base, base + table, base + tags, flags, entries};
    return true;
}

static const Tag *find_tag(const Tag *tags, unsigned count, unsigned kind)
{
    for (unsigned i = 0; i < count; ++i)
        if (tags[i].kind == kind)
            return &tags[i];
    return NULL;
}

static Format track_format(Archive a, const Tag *tags, unsigned count)
{
    const Tag *format = find_tag(tags, count, 0x76);
    if (format && format->end <= a.size && format->end - format->start >= 30) {
        const uint8_t *p = a.data + format->start;
        return (Format){p[28], le(p + 8, 2), p[29], le(p, 4), true};
    }
    format = find_tag(tags, count, 0xfa);
    if (format && format->end <= a.size && format->end - format->start >= 20) {
        const uint8_t *p = a.data + format->start;
        return (Format){p[19], le(p + 8, 2), 1, le(p, 4), true};
    }
    format = find_tag(tags, count, 0x48);
    if (format && format->end <= a.size && format->end - format->start >= 12) {
        const uint8_t *p = a.data + format->start;
        unsigned channels = le(p + 8, 4);
        if (channels > 0 && channels <= 64 && 12 + 16 * channels <= format->end - format->start) {
            Format result = {p[24], le(p + 22, 2), channels, le(p + 16, 4), true};
            for (unsigned i = 1; i < channels; ++i) {
                const uint8_t *channel = p + 16 * i;
                if (channel[24] != result.codec || le(channel + 22, 2) != result.rate)
                    return (Format){0};
                uint32_t samples = le(channel + 16, 4);
                if (samples > result.samples)
                    result.samples = samples;
            }
            return result;
        }
    }
    return (Format){0};
}

static bool packets_for_track(Mux *mux, const uint8_t *data, size_t size, Format *format, size_t *count)
{
    uint64_t frames_total = 0, retained_frames = 0;
    *count = 0;
    for (size_t block = 0; block < size; block += BLOCK) {
        const uint8_t *bytes = data + block;
        unsigned offset = 0;
        while (offset + 2 <= BLOCK) {
            unsigned length = le(bytes + offset, 2);
            if (length == 0)
                break;
            if (length > BLOCK - offset - 2)
                return false;
            const uint8_t *packet = bytes + offset + 2;
            int frames = mux->duration(packet, (int)length, 48000);
            if (frames <= 0 || (format->channels == 1 && (packet[0] & 4)))
                return false;
            if (frames_total < format->samples) {
                if (*count == mux->capacity) {
                    size_t capacity = mux->capacity ? 2 * mux->capacity : 256;
                    Packet *grown = realloc(mux->packets, capacity * sizeof(*grown));
                    if (!grown)
                        system_fail("allocating Opus packet list");
                    mux->packets = grown;
                    mux->capacity = capacity;
                }
                mux->packets[(*count)++] = (Packet){packet, length, (unsigned)frames};
                retained_frames += (unsigned)frames;
            }
            frames_total += (unsigned)frames;
            offset += 2 + length;
        }
        for (; offset < BLOCK; ++offset)
            if (bytes[offset] != 0)
                return false;
    }
    uint32_t samples = format->samples;
    if (format->channels == 2 && samples % 2 == 0 && samples > frames_total)
        samples /= 2;
    if (frames_total < samples || frames_total - samples > 5760)
        return false;
    while (*count && retained_frames - mux->packets[*count - 1].frames >= samples)
        retained_frames -= mux->packets[--*count].frames;
    format->samples = samples;
    return *count != 0;
}

static size_t align_block(size_t value)
{
    return (value + BLOCK - 1) & ~(size_t)(BLOCK - 1);
}

static bool parse_multichannel(Archive archive, Mux *mux, const uint8_t *data, size_t size,
                               const Tag *format_tag, Format format, MultiChannel *channels,
                               unsigned *frame_values)
{
    unsigned streams = format.channels;
    if (!mux->duration || streams < 1 || streams > MAX_STREAMS || format.codec != 11 ||
        format.rate != 48000 || !format.samples || !size || size % BLOCK ||
        format_tag->end - format_tag->start < 12 + 16 * streams)
        return false;

    const uint8_t *fmt = archive.data + format_tag->start;
    size_t stride = le(fmt + 4, 4);
    if (!stride || stride % BLOCK || le(fmt + 8, 4) != streams ||
        le(fmt, 4) != 1 + (size - 1) / stride)
        return false;
    for (unsigned ch = 0; ch < streams; ++ch) {
        const uint8_t *description = fmt + 16 + 16 * ch;
        if (le(description, 4) != format.samples || le(description + 6, 2) != 48000 ||
            description[8] != 11)
            return false;
    }

    uint64_t logical_end[MAX_STREAMS] = {0};
    unsigned stereo_flag = 0;
    *frame_values = PACKET_SAMPLES;
    for (size_t block = 0; block < size; block += stride) {
        const uint8_t *base = data + block;
        size_t block_size = size - block < stride ? size - block : stride;
        size_t seek = CHANNEL_RECORD * streams;
        if (seek > block_size)
            goto invalid;
        size_t groups = 0;
        for (unsigned ch = 0; ch < streams; ++ch) {
            uint32_t count = le(base + CHANNEL_RECORD * ch + 4, 4);
            if (!count || count > block_size / BLOCK - groups)
                goto invalid;
            groups += count;
        }
        size_t cursor = align_block(seek + 4 * groups);
        if (cursor > block_size || groups > (block_size - cursor) / BLOCK)
            goto invalid;

        if (!block) {
            stereo_flag = base[cursor + 2] & 4;
            if (stereo_flag)
                *frame_values *= 2;
        }
        for (unsigned ch = 0; ch < streams; ++ch) {
            const uint8_t *record = base + CHANNEL_RECORD * ch;
            uint32_t count = le(record + 4, 4);
            uint32_t overlap = le(record + 8, 4);
            uint32_t samples = le(record + 12, 4);
            uint64_t position = le(base + seek, 4);
            if (position + overlap != logical_end[ch] || position % *frame_values ||
                overlap % *frame_values || !samples || samples % *frame_values)
                goto invalid;
            logical_end[ch] = position + samples;
            MultiChannel *track = channels + ch;
            for (uint32_t group = 0; group < count; ++group, seek += 4, cursor += BLOCK) {
                if (le(base + seek, 4) != position)
                    goto invalid;
                const uint8_t *sub = base + cursor;
                unsigned offset = 0;
                while (offset + 2 <= BLOCK) {
                    unsigned length = le(sub + offset, 2);
                    if (!length)
                        break;
                    if (length > MAX_OPUS_PACKET || length > BLOCK - offset - 2)
                        goto invalid;
                    const uint8_t *packet = sub + offset + 2;
                    if ((packet[0] & 7) != stereo_flag ||
                        mux->duration(packet, (int)length, 48000) != PACKET_SAMPLES)
                        goto invalid;
                    // Physical blocks can repeat packets from either side of a logical boundary.
                    size_t index = position / *frame_values;
                    if (index < track->count) {
                        MultiPacket prior = track->packets[index];
                        if (prior.bytes != length || memcmp(prior.data, packet, length) != 0)
                            goto invalid;
                    } else if (index == track->count) {
                        if (track->count == track->capacity) {
                            size_t capacity = track->capacity ? track->capacity * 2 : 256;
                            MultiPacket *grown = realloc(track->packets, capacity * sizeof(*grown));
                            if (!grown)
                                system_fail("allocating multichannel Opus packets");
                            track->packets = grown;
                            track->capacity = capacity;
                        }
                        track->packets[track->count++] = (MultiPacket){packet, length};
                    } else {
                        goto invalid;
                    }
                    position += *frame_values;
                    offset += 2 + length;
                }
                for (; offset < BLOCK; ++offset)
                    if (sub[offset] != 0)
                        goto invalid;
            }
            if (block + block_size == size && position != logical_end[ch] &&
                position != logical_end[ch] + *frame_values)
                goto invalid;
        }
        for (; cursor < block_size; ++cursor)
            if (base[cursor] != 0)
                goto invalid;
    }

    if (stereo_flag && format.samples % 2)
        goto invalid;
    uint32_t samples = stereo_flag ? format.samples / 2 : format.samples;
    uint64_t total = (uint64_t)channels[0].count * PACKET_SAMPLES;
    if (samples > total || total > (uint64_t)samples + 5760)
        goto invalid;
    for (unsigned ch = 1; ch < streams; ++ch)
        if (channels[ch].count != channels[0].count)
            goto invalid;
    channels[0].count = ((uint64_t)samples + PACKET_SAMPLES - 1) / PACKET_SAMPLES;
    return true;

invalid:
    for (unsigned ch = 0; ch < streams; ++ch)
        free(channels[ch].packets);
    return false;
}

static size_t ogg_page(FILE *out, const Mux *mux, const Packet *packets, size_t count,
                       unsigned flags, uint64_t granule, unsigned serial, unsigned sequence)
{
    uint8_t page[27 + 255 + 255 * 255];
    memset(page, 0, 27);
    size_t segments = 0;
    memcpy(page, "OggS", 4);
    page[5] = (uint8_t)flags;
    put(page, 6, granule, 8);
    put(page, 14, serial, 4);
    put(page, 18, sequence, 4);
    for (size_t i = 0; i < count; ++i) {
        unsigned length = packets[i].bytes;
        if (segments + length / 255 + 1 > 255)
            fail("Ogg page exceeds lacing bounds");
        for (; length >= 255; length -= 255)
            page[27 + segments++] = 255;
        page[27 + segments++] = (uint8_t)length;
    }
    page[26] = (uint8_t)segments;
    size_t bytes = 27 + segments;
    for (size_t i = 0; i < count; ++i) {
        memcpy(page + bytes, packets[i].data, packets[i].bytes);
        bytes += packets[i].bytes;
    }
    uint32_t crc = 0;
    for (size_t i = 0; i < bytes; ++i)
        crc = (crc << 8) ^ mux->crc[(crc >> 24) ^ page[i]];
    put(page, 22, crc, 4);
    write_bytes(out, page, bytes);
    return bytes;
}

static size_t write_ogg_headers(FILE *out, const Mux *mux, unsigned rate, unsigned channels,
                                unsigned streams, unsigned coupled, unsigned serial)
{
    uint8_t head[21 + 2 * MAX_STREAMS] = "OpusHead";
    uint8_t tags[33] = "OpusTags";
    head[8] = 1;
    head[9] = (uint8_t)channels;
    put(head, 12, rate, 4);
    unsigned head_size = 19;
    if (streams > 1) {
        head[18] = channels <= 8 ? 1 : 255;
        head[19] = (uint8_t)streams;
        head[20] = (uint8_t)coupled;
        for (unsigned ch = 0; ch < channels; ++ch)
            head[21 + ch] = (uint8_t)ch;
        head_size = 21 + channels;
    }
    put(tags, 8, 17, 4);
    memcpy(tags + 12, "Opus packet remux", 17);
    Packet identification = {head, head_size, 0};
    Packet comments = {tags, sizeof(tags), 0};
    size_t bytes = ogg_page(out, mux, &identification, 1, 2, 0, serial, 0);
    return bytes + ogg_page(out, mux, &comments, 1, 0, 0, serial, 1);
}

static size_t write_ogg(FILE *out, const Mux *mux, size_t count, Format format, unsigned serial)
{
    size_t bytes = write_ogg_headers(out, mux, format.rate, format.channels, 1, 0, serial);
    uint64_t position = 0;
    unsigned sequence = 2;
    for (size_t at = 0; at < count;) {
        size_t batch = count - at > PAGE_PACKETS ? PAGE_PACKETS : count - at;
        for (size_t i = 0; i < batch; ++i)
            position += mux->packets[at + i].frames;
        bytes += ogg_page(out, mux, mux->packets + at, batch, at + batch == count ? 4 : 0,
                          position < format.samples ? position : format.samples, serial, sequence++);
        at += batch;
    }
    return bytes;
}

static size_t write_ogg_multi(FILE *out, const Mux *mux, const MultiChannel *channels,
                              Format format, unsigned frame_values, unsigned serial)
{
    unsigned streams = format.channels;
    unsigned coupled = frame_values == 2 * PACKET_SAMPLES ? streams : 0;
    size_t bytes = write_ogg_headers(out, mux, format.rate, streams + coupled, streams, coupled, serial);
    unsigned per_page = streams > 4 ? 5 : streams > 1 ? 10 : PAGE_PACKETS;
    uint32_t samples = coupled ? format.samples / 2 : format.samples;
    uint64_t position = 0;
    unsigned sequence = 2;
    size_t count = channels[0].count;
    for (size_t at = 0; at < count;) {
        size_t batch = count - at > per_page ? per_page : count - at;
        Packet joined[PAGE_PACKETS];
        uint8_t storage[10][MAX_JOINED_PACKET];
        for (size_t i = 0; i < batch; ++i) {
            if (streams == 1) {
                MultiPacket packet = channels[0].packets[at + i];
                joined[i] = (Packet){packet.data, packet.bytes, PACKET_SAMPLES};
                continue;
            }
            uint8_t *out_packet = storage[i];
            for (unsigned ch = 0; ch < streams; ++ch) {
                MultiPacket packet = channels[ch].packets[at + i];
                if (ch + 1 < streams) {
                    // All but the last stream use RFC 6716 self-delimiting framing.
                    *out_packet++ = *packet.data++;
                    --packet.bytes;
                    if (packet.bytes < 252) {
                        *out_packet++ = (uint8_t)packet.bytes;
                    } else {
                        unsigned first = 252 + (packet.bytes & 3);
                        *out_packet++ = (uint8_t)first;
                        *out_packet++ = (uint8_t)((packet.bytes - first) / 4);
                    }
                }
                memcpy(out_packet, packet.data, packet.bytes);
                out_packet += packet.bytes;
            }
            joined[i] = (Packet){storage[i], (unsigned)(out_packet - storage[i]), PACKET_SAMPLES};
        }
        position += (uint64_t)PACKET_SAMPLES * batch;
        bytes += ogg_page(out, mux, joined, batch, at + batch == count ? 4 : 0,
                          position < samples ? position : samples, serial, sequence++);
        at += batch;
    }
    return bytes;
}

static size_t write_xma_header(FILE *out, size_t size, Format format)
{
    uint8_t header[80] = {0};
    memcpy(header, "RIFF", 4);
    memcpy(header + 8, "WAVEfmt ", 8);
    memcpy(header + 72, "data", 4);
    put(header, 4, 72 + size, 4);
    put(header, 16, 52, 4);
    put(header, 20, 0x166, 2);
    put(header, 22, 1, 2);
    put(header, 24, format.rate, 4);
    put(header, 28, format.rate / 2, 4);
    put(header, 32, 2, 2);
    put(header, 34, 16, 2);
    put(header, 36, 34, 2);
    put(header, 38, 1, 2);
    put(header, 40, 4, 4);
    put(header, 44, 2 * format.samples, 4);
    put(header, 48, 0x8000, 4);
    header[69] = 4;
    put(header, 70, (size + 0x7fff) / 0x8000, 2);
    put(header, 76, size, 4);
    write_bytes(out, header, sizeof(header));
    return sizeof(header);
}

static void export_track(Archive a, Bank bank, unsigned number, unsigned id, unsigned occurrence,
                         const Tag *tags, unsigned tag_count, Tag payload, Format format,
                         bool raw, Mux *mux, FILE *manifest, char *path, size_t prefix, Totals *totals)
{
    char name[48] = "";
    const char *status;
    size_t written = 0;
    if (payload.start == payload.end && payload.end <= a.size) {
        status = "empty";
        ++totals->empty;
    } else if (payload.end > a.size) {
        status = "out-of-bounds";
        ++totals->invalid;
    } else {
        size_t size = payload.end - payload.start, packets = 0;
        const uint8_t *data = a.data + payload.start;
        Format wrapped = format;
        MultiChannel multi[MAX_STREAMS] = {{0}};
        unsigned frame_values;
        const Tag *stream_format = find_tag(tags, tag_count, 0x48);
        bool multi_opus = !raw && payload.kind == 0x55 && stream_format &&
                          parse_multichannel(a, mux, data, size, stream_format, format, multi, &frame_values);
        bool opus = mux->duration && payload.kind == 0x55 && format.codec == 11 && format.rate &&
                    (format.channels == 1 || format.channels == 2) && format.samples &&
                    size % BLOCK == 0 && !stream_format && packets_for_track(mux, data, size, &wrapped, &packets);
        bool xma = !raw && payload.kind == 0x55 && format.codec == 5 && format.rate &&
                   format.channels == 1 && format.samples && format.samples < 0x80000000 &&
                   size % BLOCK == 0;
        const char *ext = opus || multi_opus ? "opus" : xma ? "xma" : payload.kind == 0x68 ? "mid" : "bin";
        if (occurrence > 1)
            snprintf(name, sizeof(name), "%05u-%02x-%02u.%s", number, payload.kind, occurrence, ext);
        else
            snprintf(name, sizeof(name), "%05u-%02x.%s", number, payload.kind, ext);
        strcpy(path + prefix, name);
        FILE *out = fopen(path, "wbx");
        if (!out)
            system_fail(path);
        if (multi_opus) {
            written = write_ogg_multi(out, mux, multi, format, frame_values, number);
            status = "opus";
            ++totals->opus;
            for (unsigned ch = 0; ch < format.channels; ++ch)
                free(multi[ch].packets);
        } else if (opus) {
            written = write_ogg(out, mux, packets, wrapped, number);
            status = "opus";
            ++totals->opus;
        } else {
            if (xma)
                written = write_xma_header(out, size, format);
            write_bytes(out, data, size);
            written += size;
            status = xma ? "xma2" : "raw";
            if (xma)
                ++totals->xma2;
            else
                ++totals->raw;
        }
        if (fclose(out) != 0)
            system_fail(path);
    }

    totals->bytes += written;
    fprintf(manifest, "0x%zx\t0x%x\t%u\t0x%x\t0x%x\t%u\t",
            bank.base, bank.flags, number, id, payload.kind, occurrence);
    if (format.known) {
        const char *codec_name = format.codec == 5 ? "XMA2" : format.codec == 11 ? "Opus" : "";
        fprintf(manifest, "0x%x\t%s\t%u\t%u\t%" PRIu32,
                format.codec, codec_name, format.rate, format.channels, format.samples);
    } else {
        fputs("\t\t\t\t", manifest);
    }
    fprintf(manifest, "\t0x%zx\t0x%zx\t%s\t%zu\t", payload.start, payload.end, status, written);
    if (*name)
        fprintf(manifest, "%09zX/%s", bank.base, name);
    fputc('\t', manifest);
    for (unsigned i = 0; i < tag_count; ++i) {
        Tag tag = tags[i].kind == payload.kind ? payload : tags[i];
        fprintf(manifest, "%s%02x:0x%zx-0x%zx", i ? "," : "", tag.kind, tag.start, tag.end);
    }
    fputc('\n', manifest);
}

static void extract_bank(Archive a, Bank bank, bool raw, Mux *mux, FILE *manifest,
                         char *path, size_t root_end, Totals *totals)
{
    sprintf(path + root_end, "/%09zX", bank.base);
    size_t prefix = strlen(path) + 1;
    bool directory_created = false;
    size_t at = bank.tags;
    for (unsigned i = 0; i < bank.entries; ++i) {
        uint32_t entry = le(a.data + bank.table + 4 * i, 4);
        Tag tags[MAX_TAGS], payloads[MAX_TAGS];
        unsigned tag_count = 0, payload_count = 0;
        for (unsigned j = 0; j < (entry >> 29); ++j, at += 8) {
            Tag tag = read_tag(a, bank.base, at);
            unsigned slot = 0;
            while (slot < tag_count && tags[slot].kind < tag.kind)
                ++slot;
            if (slot == tag_count || tags[slot].kind != tag.kind) {
                memmove(tags + slot + 1, tags + slot, (tag_count - slot) * sizeof(*tags));
                ++tag_count;
            }
            tags[slot] = tag;
            if (tag.kind == 0x55 || (tag.kind == 0x68 && tag.end - tag.start >= 14 &&
                tag.end <= a.size && memcmp(a.data + tag.start, "MThd\0\0\0\6", 8) == 0))
                payloads[payload_count++] = tag;
        }
        if (!payload_count)
            continue;
        Format format = track_format(a, tags, tag_count);
        unsigned audio = 0, midi = 0;
        for (unsigned j = 0; j < payload_count; ++j) {
            Tag payload = payloads[j];
            if (!directory_created && payload.start < payload.end && payload.end <= a.size) {
                if (mkdir(path, 0777) != 0)
                    system_fail(path);
                path[prefix - 1] = '/';
                directory_created = true;
            }
            unsigned occurrence = payload.kind == 0x55 ? ++audio : ++midi;
            export_track(a, bank, i + 1, entry & 0x1fffffff, occurrence, tags, tag_count,
                         payload, format, raw, mux, manifest, path, prefix, totals);
        }
    }
    if (ferror(manifest) || fflush(manifest) == EOF)
        system_fail("writing manifest");
    ++totals->banks;
}

static void print_totals(FILE *out, Totals t)
{
    fprintf(out, "banks=%" PRIuFAST64 " opus=%" PRIuFAST64 " xma2=%" PRIuFAST64
            " raw=%" PRIuFAST64 " empty=%" PRIuFAST64 " out-of-bounds=%" PRIuFAST64
            " bytes=%" PRIuFAST64 "\n", t.banks, t.opus, t.xma2, t.raw, t.empty, t.invalid, t.bytes);
}

int main(int argc, char **argv)
{
    const char *source = NULL, *output = NULL;
    bool raw = false, selected = false;
    size_t base = 0;
    for (int i = 1; i < argc; ++i) {
        if (strcmp(argv[i], "--help") == 0) {
            puts("usage: rpf9_extract_audio ARCHIVE --out-dir DIR [--bank OFFSET] [--raw]");
            return 0;
        }
        if (strcmp(argv[i], "--raw") == 0) {
            raw = true;
        } else if (strcmp(argv[i], "--out-dir") == 0 && i + 1 < argc) {
            output = argv[++i];
        } else if (strcmp(argv[i], "--bank") == 0 && i + 1 < argc) {
            const char *value = argv[++i];
            char *end;
            errno = 0;
            uintmax_t parsed = strtoumax(value, &end, 0);
            if (value[0] == '-' || *end || end == value || errno || parsed > SIZE_MAX)
                fail("invalid bank offset");
            base = (size_t)parsed;
            selected = true;
        } else if (!source && argv[i][0] != '-') {
            source = argv[i];
        } else {
            fail("expected ARCHIVE --out-dir DIR [--bank OFFSET] [--raw]");
        }
    }
    if (!source || !output || !*output)
        fail("expected ARCHIVE --out-dir DIR [--bank OFFSET] [--raw]");
    int fd = open(source, O_RDONLY);
    if (fd < 0)
        system_fail(source);
    struct stat statbuf;
    if (fstat(fd, &statbuf) != 0)
        system_fail(source);
    if (statbuf.st_size < 4 || (uintmax_t)statbuf.st_size > SIZE_MAX - 0x20000000)
        fail("archive size cannot be mapped");
    size_t size = (size_t)statbuf.st_size;
    void *mapping = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
    if (mapping == MAP_FAILED)
        system_fail(source);
    close(fd);
    Archive a = {mapping, size};
    if (memcmp(a.data, "9FPR", 4) != 0)
        fail("expected an RPF9 archive with on-disk magic 9FPR");
    Bank bank;
    if (selected && !bank_at(a, base, &bank))
        fail("no readable ADAT bank at the selected offset");

    Mux mux = {0};
    void *opus_library = NULL;
    if (!raw) {
        opus_library = dlopen("libopus.so.0", RTLD_LAZY | RTLD_LOCAL);
        if (opus_library) {
            void *symbol = dlsym(opus_library, "opus_packet_get_nb_samples");
            _Static_assert(sizeof(symbol) == sizeof(mux.duration), "POSIX function pointer size");
            if (symbol)
                memcpy(&mux.duration, &symbol, sizeof(symbol));
        }
        if (!mux.duration)
            fputs("libopus unavailable. Exporting original payloads.\n", stderr);
        for (unsigned i = 0; i < 256; ++i) {
            uint32_t crc = (uint32_t)i << 24;
            for (unsigned bit = 0; bit < 8; ++bit)
                crc = (crc << 1) ^ ((crc & 0x80000000) ? 0x04c11db7 : 0);
            mux.crc[i] = crc;
        }
    }

    size_t root_end = strlen(output);
    while (root_end > 1 && output[root_end - 1] == '/')
        --root_end;
    char *path = malloc(root_end + 128);
    if (!path)
        system_fail("allocating output path");
    memcpy(path, output, root_end);
    path[root_end] = '\0';
    make_output(path);
    strcpy(path + root_end, "/manifest.tsv");
    FILE *manifest = fopen(path, "wx");
    if (!manifest)
        system_fail(path);
    fputs("bank\tbank_flags\tentry\tentry_id\tpayload_tag\ttag_occurrence\tcodec_id\tcodec_name\t"
          "sample_rate_hz\tchannels\tdeclared_samples\tstart\tend_exclusive\tstatus\toutput_bytes\tpath\ttag_ranges\n",
          manifest);

    Totals totals = {0};
    if (selected) {
        extract_bank(a, bank, raw, &mux, manifest, path, root_end, &totals);
    } else {
        for (size_t offset = 0; offset + 16 <= size; offset += 16) {
            if (a.data[offset] != 'A' || !bank_at(a, offset, &bank))
                continue;
            extract_bank(a, bank, raw, &mux, manifest, path, root_end, &totals);
            if (totals.banks % 100 == 0)
                print_totals(stderr, totals);
        }
    }
    if (fclose(manifest) != 0)
        system_fail("closing manifest");
    free(path);
    free(mux.packets);
    if (opus_library)
        dlclose(opus_library);
    munmap(mapping, size);
    if (!totals.banks)
        fail("no readable ADAT banks found");
    print_totals(stdout, totals);
    return 0;
}
