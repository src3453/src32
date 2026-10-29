"""Small FPGA console startup font; no wide-font ROM/storage dependency."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def write_header(path):
    font = (ROOT / "assets/UNSCII.F08").read_bytes()
    if len(font) != 2048:
        raise ValueError("UNSCII.F08 must contain 256 8x8 CP437 glyphs")
    # Match the PCG board demo: two identical 8x8 banks, doubled 8x16 rows.
    fonts = font * 2 + bytes(b for row in font for b in (row, row))
    palette = bytes(v for i in range(64)
                    for v in ((i >> 4) * 85, ((i >> 2) & 3) * 85, (i & 3) * 85))
    source = "/* Generated from UNSCII.F08; see assets/UNSCII-LICENSE.txt. */\n"
    for name, data in (("fonts", fonts), ("palette", palette)):
        source += f"static const uint8_t tang_{name}[{len(data)}] = {{\n"
        source += ",\n".join(",".join(str(b) for b in data[n:n+32])
                             for n in range(0, len(data), 32)) + "\n};\n"
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(source, encoding="utf-8")
    return path
