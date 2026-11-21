"""Shared extraction primitives for Double Commander integrations."""

import zlib

RAW_EXTENSIONS = {"bk2", "awc", "png", "wav", "riff", "webp", "avi"}
BUFFER_SIZE = 1024 * 1024


def copy_stream(archive, output, offset, stored, decoded, ext):
    with archive.open("rb") as source:
        source.seek(offset)
        unpacker = None if ext in RAW_EXTENSIONS else zlib.decompressobj()
        written = 0
        while stored:
            data = source.read(min(BUFFER_SIZE, stored))
            if not data:
                raise RuntimeError("Truncated archive entry")
            stored -= len(data)
            if unpacker:
                data = unpacker.decompress(data, decoded - written + 1)
                if len(data) > decoded - written:
                    raise RuntimeError("Archive entry exceeds its indexed size")
            output.write(data)
            written += len(data)
        if unpacker and (not unpacker.eof or unpacker.unused_data or written != decoded):
            raise RuntimeError("Invalid compressed entry")
