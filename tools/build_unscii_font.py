"""Convert upstream UNSCII-8 Unicode hex data to a 256-glyph CP437 PCG bank."""
from pathlib import Path
import argparse

# CP437 graphical characters occupy control-code slots in the original CGA bank.
CONTROLS = "\0☺☻♥♦♣♠•◘○◙♂♀♪♫☼►◄↕‼¶§▬↨↑↓→←∟↔▲▼"


def convert(source: str) -> bytes:
    glyphs = {}
    for line in source.splitlines():
        if not line.strip():
            continue
        code, bitmap = line.split(":")
        glyphs[int(code, 16)] = bytes.fromhex(bitmap)
    result = bytearray()
    for code in range(256):
        char = CONTROLS[code] if code < 32 else "⌂" if code == 127 else bytes([code]).decode("cp437")
        glyph = glyphs.get(ord(char), glyphs[ord("?")])
        # Upstream emits double-width empty U+0000; PCG's NUL must remain blank.
        if code == 0:
            glyph = bytes(8)
        if len(glyph) != 8:
            raise ValueError(f"CP437 {code:#x} maps to a non-8x8 glyph: U+{ord(char):04X}")
        result.extend(glyph)
    return bytes(result)


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    data = convert((root / "assets/unscii-8.hex").read_text(encoding="utf-8"))
    target = root / "assets/UNSCII.F08"
    if args.check:
        if target.read_bytes() != data:
            raise SystemExit("UNSCII.F08 is stale; run tools/build_unscii_font.py")
    else:
        target.write_bytes(data)
    print(f"UNSCII CP437 bank: {len(data)} bytes")
