"""FS2 host utility tests (doc/spec_FS2.md).

    python -m unittest discover -s tools/fs2 -v
"""
import contextlib
import io
from pathlib import Path
import struct
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fs2  # noqa: E402

STAMP = (1_700_000_000, 123)


def volume(blocks=2880, boot_blocks=0, inodes=256):
    return fs2.Volume(fs2.format_image(blocks, boot_blocks, inodes, created=STAMP))


class LayoutTests(unittest.TestCase):
    def test_contract_example(self):
        # §12: 2880 sectors, boot_blocks=0, inode_count=256.
        vol = volume()
        self.assertEqual({k: vol.sb[k] for k in ("bitmap_start", "bitmap_blocks", "inode_start",
                                                  "inode_blocks", "data_start")},
                         dict(bitmap_start=2, bitmap_blocks=1, inode_start=3, inode_blocks=64, data_start=67))
        free = len(vol.free_blocks())
        vol.put("/a", bytes(600), stamp=STAMP)
        # 2 data blocks plus 3 blocks for root's first 1040-byte entry.
        self.assertEqual(free - len(vol.free_blocks()), 5)
        self.assertEqual(fs2.check_image(vol.img), [])
        for _ in range(254):
            vol.create(f"/f{_}", fs2.FILE, stamp=STAMP)
        with self.assertRaises(fs2.Fs2Error) as error:
            vol.create("/last-but-one-too-many", fs2.FILE, stamp=STAMP)
        self.assertEqual(error.exception.code, "LIMIT")

    def test_minimum_volume_and_format_state(self):
        img = fs2.format_image(4, 0, 1, created=STAMP)
        self.assertEqual(fs2.check_image(img), [])
        vol = fs2.Volume(img)
        root = vol.read_inode(1)
        self.assertEqual((root["type"], root["uid"], root["mode"], root["parent"], root["size"]),
                         (fs2.DIR, 0, 0x003D, 1, 0))
        self.assertEqual(img[510:512], b"\x55\xaa")
        self.assertEqual(img[512:516], b"FS2\0")
        with self.assertRaises(fs2.Fs2Error):
            fs2.format_image(3, 0, 1)

    def test_bitmap_bits_are_msb_first(self):
        vol = volume()
        bitmap = vol.sb["bitmap_start"] * fs2.BLOCK
        self.assertEqual(vol.img[bitmap:bitmap + 8], b"\xff" * 8 + b"")
        self.assertEqual(vol.img[bitmap + 8], 0xE0)  # blocks 64..66 used, 67.. free


class DataTests(unittest.TestCase):
    def test_boundary_sizes_round_trip(self):
        vol = volume()
        for size in (0, 1, 508, 509, 1016, 1017):
            data = bytes((i * 7) & 0xFF for i in range(size))
            inode = vol.put(f"/s{size}", data, stamp=STAMP)
            self.assertEqual(inode["block_count"], -(-size // 508))
            self.assertEqual(vol.read(vol.resolve(f"/s{size}")), data)
        self.assertEqual(fs2.check_image(vol.img), [])

    def test_replace_shrink_and_fragmented_allocation(self):
        vol = volume(blocks=100, inodes=4)
        vol.put("/a", bytes(508 * 3), stamp=STAMP)
        vol.put("/b", b"x" * 508, stamp=STAMP)
        vol.put("/a", b"y", stamp=STAMP)  # frees two blocks between a and b
        vol.put("/c", b"z" * 1500, stamp=STAMP)
        self.assertEqual(vol.read(vol.resolve("/c")), b"z" * 1500)
        self.assertEqual(vol.read(vol.resolve("/a")), b"y")
        self.assertEqual(fs2.check_image(vol.img), [])

    def test_hole_is_zero_filled(self):
        vol = volume()
        inode = vol.put("/h", b"ab", stamp=STAMP)
        vol.write(inode, 1000, b"cd")
        vol.write_inode(inode["number"], inode)
        self.assertEqual(vol.read(vol.resolve("/h")), b"ab" + bytes(998) + b"cd")
        self.assertEqual(fs2.check_image(vol.img), [])

    def test_no_space(self):
        vol = volume(blocks=80, inodes=4)  # data_start 4, 76 data blocks
        with self.assertRaises(fs2.Fs2Error) as error:
            vol.put("/big", bytes(508 * 80), stamp=STAMP)
        self.assertEqual(error.exception.code, "NO_SPACE")


class NameTests(unittest.TestCase):
    def test_lengths(self):
        vol = volume()
        vol.put("/" + "あ" * 256, b"", stamp=STAMP)
        vol.put("/" + "\U0001F600" * 256, b"", stamp=STAMP)  # 1024 bytes
        for name in ("a" * 257, "あ" * 257):
            with self.assertRaises(fs2.Fs2Error) as error:
                vol.put("/" + name, b"", stamp=STAMP)
            self.assertEqual(error.exception.code, "NAME_TOO_LONG")
        self.assertEqual(fs2.check_image(vol.img), [])

    def test_invalid_names(self):
        for name in (".", "..", "a\x01", "a\x7f", "a\x85", "\ud800"):
            with self.assertRaises(fs2.Fs2Error):
                fs2.check_name(name)
        for name in ("a\\b", " a. ", "日本語.txt"):
            fs2.check_name(name)

    def test_case_folding(self):
        vol = volume()
        vol.put("/Readme", b"1", stamp=STAMP)
        vol.put("/Straße", b"2", stamp=STAMP)
        vol.put("/é", b"3", stamp=STAMP)
        vol.put("/é", b"4", stamp=STAMP)  # no normalization: a different name
        for clash in ("/README", "/STRASSE"):
            with self.assertRaises(fs2.Fs2Error) as error:
                vol.create(clash, fs2.FILE, stamp=STAMP)
            self.assertEqual(error.exception.code, "EXISTS")
        self.assertEqual(vol.read(vol.resolve("/readme")), b"1")
        # Unicode 16.0.0 additions are folded even where the host Python is older.
        self.assertEqual(fs2.fold("Ɤ"), "ɤ")
        self.assertEqual(fs2.fold("İ"), "i̇")  # F, not the Turkic T mapping

    def test_case_only_rename_and_same_name(self):
        vol = volume()
        vol.put("/readme", b"1", stamp=STAMP)
        vol.rename("/readme", "/README", stamp=STAMP)
        self.assertEqual([e[2] for e in vol.entries(vol.read_inode(1))], ["README"])
        vol.rename("/README", "/README", stamp=STAMP)
        vol.put("/other", b"", stamp=STAMP)
        with self.assertRaises(fs2.Fs2Error):
            vol.rename("/other", "/readme", stamp=STAMP)


class TreeTests(unittest.TestCase):
    def test_paths_mkdir_move_remove(self):
        vol = volume()
        vol.mkdir("/a/b/c", parents=True, stamp=STAMP)
        vol.put("/a/b/c/f", b"data", stamp=STAMP)
        self.assertEqual(vol.read(vol.resolve("/a/./b/../b/c/f")), b"data")
        self.assertEqual(vol.resolve("/..")["number"], 1)
        with self.assertRaises(fs2.Fs2Error) as error:
            vol.resolve("/missing/../a")
        self.assertEqual(error.exception.code, "NOT_FOUND")
        with self.assertRaises(fs2.Fs2Error):
            vol.resolve("/a/b/c/f/")
        with self.assertRaises(fs2.Fs2Error):
            vol.rename("/a", "/a/b/c/a", stamp=STAMP)
        vol.rename("/a/b/c", "/c", stamp=STAMP)
        self.assertEqual(vol.resolve("/c")["parent"], 1)
        with self.assertRaises(fs2.Fs2Error) as error:
            vol.remove("/c", stamp=STAMP)
        self.assertEqual(error.exception.code, "NOT_EMPTY")
        vol.remove("/c/f", stamp=STAMP)
        vol.remove("/c", stamp=STAMP)
        self.assertEqual(fs2.check_image(vol.img), [])
        # Directories keep their freed slots: / has 2 entries (5 blocks), /a and /a/b 1 each (3 blocks).
        self.assertEqual(len(vol.free_blocks()), 2880 - 67 - 5 - 3 - 3)

    def test_attributes(self):
        vol = volume()
        vol.put("/f", b"", stamp=STAMP)
        vol.set_attributes("/f", mode=0x003D, uid=1000, stamp=STAMP)
        inode = vol.resolve("/f")
        self.assertEqual((inode["mode"], inode["uid"]), (0x003D, 1000))
        for kwargs in ({"mode": 0x40}, {"uid": 0xFFFFFFFF}):
            with self.assertRaises(fs2.Fs2Error):
                vol.set_attributes("/f", **kwargs)


class CheckTests(unittest.TestCase):
    def setUp(self):
        self.vol = volume()
        self.vol.put("/f", bytes(600), stamp=STAMP)

    def problems(self):
        return fs2.check_image(self.vol.img)

    def test_dirty_and_versions(self):
        self.vol.img[512 + 7] |= 1
        self.assertEqual(self.problems()[0][0], "FORMAT")
        self.assertEqual(fs2.check_image(self.vol.img, allow_dirty=True), [])
        self.vol.img[512 + 5] = 2
        self.assertEqual(self.problems()[0][0], "UNSUPPORTED")

    def test_leaked_block(self):
        self.vol.set_bit(2000, True)
        self.assertIn("not referenced", self.problems()[0][1])

    def test_shared_block(self):
        other = self.vol.put("/g", b"x", stamp=STAMP)
        other["first_block"] = self.vol.resolve("/f")["first_block"]
        self.vol.write_inode(other["number"], other)
        self.assertTrue(any("shared" in m for _, m in self.problems()))

    def test_padding_and_reserved(self):
        inode = self.vol.resolve("/f")
        last = self.vol.chain(inode)[-1]
        self.vol.img[(last + 1) * fs2.BLOCK - 1] = 1
        self.assertIn("padding", self.problems()[0][1])

    def test_bad_name(self):
        root = self.vol.read_inode(1)
        self.vol.write(root, 8, b"\xc0")  # the 1-byte name "f" becomes invalid UTF-8
        self.assertTrue(any("invalid name" in m for _, m in self.problems()))

    def test_bad_layout(self):
        struct.pack_into(">I", self.vol.img, 512 + 0x18, 3)
        self.assertIn("bitmap_start", self.problems()[0][1])


class BootTests(unittest.TestCase):
    def test_install_boot(self):
        vol = volume(boot_blocks=4)
        stage1 = bytes(range(256)) + bytes(254) + b"\x55\xaa"
        vol.install_boot(b"B" * 700, stage1)
        self.assertEqual(vol.img[:510], stage1[:510])
        self.assertEqual(vol.img[1024:1024 + 700], b"B" * 700)
        self.assertEqual(struct.unpack_from(">I", vol.img, 512 + 0x14)[0], 700)
        self.assertEqual(fs2.check_image(vol.img), [])
        with self.assertRaises(fs2.Fs2Error):
            vol.install_boot(bytes(4 * 512 + 1))
        with self.assertRaises(fs2.Fs2Error):
            vol.install_boot(b"", stage1[:510] + b"\0\0")
        vol.img[1024 + 800] = 1
        self.assertIn("boot area padding", fs2.check_image(vol.img)[0][1])


class CliTests(unittest.TestCase):
    def run_cli(self, *argv):
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
            code = fs2.main([str(a) for a in argv])
        return code, out.getvalue()

    def test_round_trip_and_failed_command_keeps_the_image(self):
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            image, host = temp / "disk.img", temp / "root"
            (host / "bin").mkdir(parents=True)
            (host / "bin" / "hello.wax").write_bytes(b"WAX")
            (host / "readme.txt").write_text("hi", encoding="utf-8")
            self.assertEqual(self.run_cli("format", image, "--blocks", 200, "--boot-blocks", 2)[0], 0)
            self.assertEqual(self.run_cli("put", image, host, "/")[0], 0)
            self.assertEqual(self.run_cli("ls", image, "/bin")[1].split()[-1], "hello.wax")
            self.assertEqual(self.run_cli("get", image, "/README.TXT", temp / "out")[0], 0)
            self.assertEqual((temp / "out").read_text(encoding="utf-8"), "hi")
            (temp / "boot").write_bytes(b"F" * 600)
            self.assertEqual(self.run_cli("install-boot", image, temp / "boot")[0], 0)
            before = image.read_bytes()
            (temp / "big").write_bytes(bytes(2000))
            self.assertEqual(self.run_cli("install-boot", image, temp / "big")[0], 1)
            self.assertEqual(self.run_cli("mkdir", image, "/bin")[0], 1)
            self.assertEqual(image.read_bytes(), before)
            self.assertEqual(self.run_cli("check", image), (0, "OK\n"))


if __name__ == "__main__":
    unittest.main()
