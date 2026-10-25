"""Regression checks for the shared PCG/SGC and Foundation glyph ROM."""
import sys
import tempfile
import unittest
from pathlib import Path

import unifont_hex_to_chr as font

ROOT = Path(__file__).resolve().parents[1]


class FontTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.base = font.convert(ROOT / "assets/unifont-18.0.01.hex.gz", None)
        cls.rom = font.convert(ROOT / "assets/unifont-18.0.01.hex.gz", font.JISKAN)
        cls.glyphs = font.jiskan_glyphs(font.JISKAN)

    def test_overlay_and_unchanged_other_slots(self):
        self.assertEqual(len(self.glyphs), 6877)
        self.assertEqual(len(self.rom), font.ROM_SIZE)
        self.assertEqual(self.rom, (ROOT / "assets/unifont-bmp.chr").read_bytes())
        for unit in range(font.CODEPOINT_COUNT):
            start = unit * 32
            expected = self.glyphs.get(unit, self.base[start:start + 32])
            self.assertEqual(self.rom[start:start + 32], expected, f"U+{unit:04X}")

    def test_jis_mapping_and_bitmap_orientation(self):
        # BDF 0x2422 (EUC-JP A4 A2) is あ. Top-to-bottom, left bit first.
        self.assertEqual(self.glyphs[0x3042].hex(),
                         "020002003ff80400044007f01c5c344424466482448245064604381c00700000")
        for text in "　、。ア日本語〜−":
            self.assertIn(ord(text), self.glyphs)
        # EUC-JP's JIS mappings must not turn into Windows CP932 aliases.
        self.assertNotIn(0xFF5E, self.glyphs)
        self.assertNotIn(0xFF0D, self.glyphs)
        for unit in (0x0041, 0xFF76, 0x4E02, 0xFFFD, 0xD800, 0xDFFF):
            self.assertNotIn(unit, self.glyphs)

    def test_foundation_uses_same_glyph_bytes(self):
        sys.path.insert(0, str(ROOT / "foundation/boot"))
        import wide_font
        self.assertTrue(set(self.glyphs).issubset(wide_font.charset()))
        # Reconstruct the emitted C words, including Unifont's U+FFFD fallback.
        import re
        header = wide_font.header_text()
        words = header.split("con_wide_glyphs[CON_WIDE_WORDS] = {", 1)[1].split("}", 1)[0]
        actual = b"".join(int(w, 16).to_bytes(4, "big") for w in re.findall(r"0x([0-9A-F]+)u", words))
        expected = b"".join(self.rom[u * 32:u * 32 + 32] for u in wide_font.charset())
        self.assertEqual(actual, expected)

    def test_invalid_bdf_rejected(self):
        original = font.JISKAN.read_text(encoding="ascii")
        variants = [
            original.replace('JISX0208.1983', 'ISO10646'),
            original.replace('CHARS 6877', 'CHARS 6878'),
            original.replace('ENCODING 8481', 'ENCODING 8482', 1),
            original.replace('BBX 16 16 0 -2', 'BBX 8 16 0 -2', 1),
            original.replace('BITMAP\n0000', 'BITMAP\nZZZZ', 1),
            original.removesuffix('ENDFONT\n'),
        ]
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "bad.bdf"
            for variant in variants:
                source.write_text(variant, encoding="ascii")
                with self.assertRaises(font.ConversionError):
                    font.jiskan_glyphs(source)


if __name__ == "__main__":
    unittest.main()
