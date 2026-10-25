#!/usr/bin/env python3
"""Build a BMP glyph ROM from Unifont with Jiskan16 JIS X 0208 glyphs."""

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
JISKAN = Path(__file__).resolve().parents[1] / "assets/jiskan16.bdf"


class ConversionError(ValueError):
    """Invalid input or output for Unifont ROM conversion."""


def jiskan_glyphs(source: Path) -> dict[int, bytes]:
    """Read X.Org's 16x16 JISX0208 BDF; map JIS positions using EUC-JP, not CP932."""
    try:
        lines = source.read_text(encoding="ascii").splitlines()
    except (OSError, UnicodeError) as exc:
        raise ConversionError(f"cannot read Jiskan16 source {source}: {exc}") from exc
    if 'CHARSET_REGISTRY "JISX0208.1983"' not in lines:
        raise ConversionError("Jiskan16 source must use JISX0208.1983 encoding")
    glyphs: dict[int, bytes] = {}
    declared = None
    records = 0
    index = 0
    try:
        while index < len(lines):
            line = lines[index]
            index += 1
            if line.startswith("CHARS "):
                declared = int(line.split()[1])
            if not line.startswith("STARTCHAR "):
                continue
            end = lines.index("ENDCHAR", index)
            record = lines[index:end]
            index = end + 1
            encoding = int(next(s for s in record if s.startswith("ENCODING ")).split()[1])
            lead, trail = encoding >> 8, encoding & 255
            if not (0x21 <= lead <= 0x7E and 0x21 <= trail <= 0x7E):
                raise ValueError("encoding outside JIS X 0208")
            unit = ord(bytes((lead + 0x80, trail + 0x80)).decode("euc_jp"))
            if unit in glyphs:
                raise ValueError(f"duplicate Unicode mapping U+{unit:04X}")
            if "BBX 16 16 0 -2" not in record or "DWIDTH 16 0" not in record:
                raise ValueError("expected 16x16 glyph with baseline offset -2")
            rows = record[record.index("BITMAP") + 1:]
            if len(rows) != 16 or any(not re.fullmatch(r"[0-9a-fA-F]{4}", row) for row in rows):
                raise ValueError("expected 16 rows of 16 bitmap bits")
            glyphs[unit] = bytes.fromhex("".join(rows))
            records += 1
    except (ValueError, StopIteration) as exc:
        raise ConversionError(f"invalid Jiskan16 BDF: {exc}") from exc
    if not glyphs or records != declared or not lines or lines[-1] != "ENDFONT":
        raise ConversionError("incomplete Jiskan16 BDF or CHARS count mismatch")
    return glyphs


def convert(source: Path, jiskan: Path | None = JISKAN) -> bytes:
    """Convert Unifont, optionally overlaying Jiskan16; None retains the base ROM."""
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
    if jiskan is not None:
        glyphs.update(jiskan_glyphs(jiskan))

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
    parser.add_argument("--jiskan", type=Path, default=JISKAN,
                        help="Jiskan16 BDF overlay (default: bundled assets/jiskan16.bdf)")
    parser.add_argument("source", type=Path, help="gzip-compressed Unifont Plane 0 .hex input")
    parser.add_argument("output", type=Path, help="32-byte-per-code-unit BMP ROM")
    args = parser.parse_args()

    try:
        rom = convert(args.source, args.jiskan)
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
