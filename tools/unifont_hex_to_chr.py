#!/usr/bin/env python3
"""Convert a GNU Unifont Plane 0 .hex file to a fixed 16x16 BMP glyph ROM."""

from __future__ import annotations

import argparse
import gzip
import os
import re
import sys
import tempfile
from pathlib import Path

CODEPOINT_COUNT = 0x10000
GLYPH_SIZE = 32
ROM_SIZE = CODEPOINT_COUNT * GLYPH_SIZE
REPLACEMENT = 0xFFFD
SURROGATE_START = 0xD800
SURROGATE_END = 0xDFFF
CODEPOINT_RE = re.compile(r"^[0-9A-Fa-f]{4,6}$")


class ConversionError(ValueError):
    """Invalid input or output for Unifont ROM conversion."""


def convert(source: Path) -> bytes:
    glyphs: dict[int, bytes] = {}
    try:
        with gzip.open(source, "rt", encoding="ascii", newline=None) as stream:
            for line_number, raw_line in enumerate(stream, 1):
                line = raw_line.rstrip("\r\n")
                codepoint_text, separator, bitmap_text = line.partition(":")
                if not separator or not CODEPOINT_RE.fullmatch(codepoint_text):
                    raise ConversionError(f"line {line_number}: malformed codepoint record")
                codepoint = int(codepoint_text, 16)
                if codepoint > 0xFFFF:
                    raise ConversionError(f"line {line_number}: non-BMP codepoint in Plane 0 source")
                if codepoint in glyphs:
                    raise ConversionError(f"line {line_number}: duplicate codepoint U+{codepoint:04X}")
                if not bitmap_text or len(bitmap_text) not in (32, 64) or not re.fullmatch(
                    r"[0-9A-Fa-f]+", bitmap_text
                ):
                    raise ConversionError(
                        f"line {line_number}: glyph must contain 8x16 or 16x16 bitmap data"
                    )
                raw = bytes.fromhex(bitmap_text)
                if len(raw) == 16:
                    glyph = bytes(byte for row in raw for byte in (row, 0))
                else:
                    glyph = raw
                glyphs[codepoint] = glyph
    except (OSError, UnicodeError) as exc:
        raise ConversionError(f"cannot read gzip source {source}: {exc}") from exc

    replacement = glyphs.get(REPLACEMENT)
    if replacement is None:
        raise ConversionError("source does not contain the U+FFFD replacement glyph")

    rom = bytearray(ROM_SIZE)
    for codepoint in range(CODEPOINT_COUNT):
        glyph = replacement if SURROGATE_START <= codepoint <= SURROGATE_END else glyphs.get(
            codepoint, replacement
        )
        start = codepoint * GLYPH_SIZE
        rom[start : start + GLYPH_SIZE] = glyph
    return bytes(rom)


def write_atomically(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temp_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix=f".{path.name}.", delete=False) as out:
            temp_path = Path(out.name)
            out.write(data)
        os.replace(temp_path, path)
    finally:
        if temp_path is not None and temp_path.exists():
            temp_path.unlink()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify an existing ROM without writing")
    parser.add_argument("source", type=Path, help="gzip-compressed Unifont Plane 0 .hex input")
    parser.add_argument("output", type=Path, help="32-byte-per-code-unit BMP ROM")
    args = parser.parse_args()

    try:
        rom = convert(args.source)
        if len(rom) != ROM_SIZE:
            raise ConversionError(f"internal ROM size error: {len(rom)} bytes, expected {ROM_SIZE}")
        if args.check:
            try:
                existing = args.output.read_bytes()
            except OSError as exc:
                raise ConversionError(f"cannot read ROM {args.output}: {exc}") from exc
            if len(existing) != ROM_SIZE:
                raise ConversionError(f"ROM size mismatch: {len(existing)} bytes, expected {ROM_SIZE}")
            if existing != rom:
                raise ConversionError(f"ROM content mismatch: {args.output}")
            print(f"verified {args.output}: {ROM_SIZE} bytes")
        else:
            write_atomically(args.output, rom)
            print(f"wrote {args.output}: {ROM_SIZE} bytes")
    except (ConversionError, OSError) as exc:
        print(f"unifont_hex_to_chr.py: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
