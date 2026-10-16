import struct
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from dsfs import (BITMAP_START, BLOCK, DIR_ENTRIES, ENTRY_SIZE, check,
                  delete_file, display_name, list_files, mkfs,
                  normalize_name, put_file, read_file, read_super, rename_file,
                  stats)

TOOL = Path(__file__).resolve().parent / "dsfs.py"
T = 1700000000


def make_image(sectors=2880):
    img = bytearray(sectors * BLOCK)
    mkfs(img, T)
    return img


def entry_at(img, index):
    offset = read_super(img)["dir_start"] * BLOCK + index * ENTRY_SIZE
    return bytes(img[offset:offset + ENTRY_SIZE])


def patch_entry(img, index, offset, data):
    base = read_super(img)["dir_start"] * BLOCK + index * ENTRY_SIZE + offset
    img[base:base + len(data)] = data


def violations(img):
    return "\n".join(check(img))


def test_mkfs_layout_matches_spec_example():
    img = make_image()
    assert check(img) == []
    sb = read_super(img)
    assert sb == {"magic": b"DSFS", "version": 1, "flags": 0, "total_blocks": 2880,
                  "bitmap_start": 2, "bitmap_blocks": 1, "dir_start": 3,
                  "dir_blocks": 16, "data_start": 19, "created": T}
    assert img[0:510] == b"\x00" * 510
    assert img[510:512] == b"\x55\xaa"
    bitmap = BITMAP_START * BLOCK
    # metadata blocks 0..18 marked used: full bytes plus bits 16..18 = 0xE0
    assert bytes(img[bitmap:bitmap + 2]) == b"\xff\xff"
    assert img[bitmap + 2] == 0xE0
    # padding bits (block >= 2880) and the last bitmap byte stay zero
    assert img[bitmap + 360] == 0
    assert img[bitmap + 511] == 0
    dir_offset = sb["dir_start"] * BLOCK
    assert img[dir_offset:dir_offset + 16 * BLOCK] == b"\x00" * (16 * BLOCK)
    assert stats(img, sb) == {"files": 0, "data_blocks": 2861,
                              "free_blocks": 2861, "leaked_blocks": 0}


def test_mkfs_rejects_tiny_and_unaligned_images():
    with pytest.raises(ValueError, match="at least 19"):
        mkfs(bytearray(18 * BLOCK), T)
    with pytest.raises(ValueError, match="512"):
        mkfs(bytearray(19 * BLOCK + 1), T)
    assert check(make_image(19)) == []  # minimum volume: zero data blocks


def test_put_get_roundtrip_and_name_normalization():
    img = make_image()
    raw = normalize_name("hello.txt")
    assert raw == b"HELLO   TXT"
    payload = bytes((i * 7 + 3) & 0xFF for i in range(600))
    info = put_file(img, raw, payload, T + 5)
    assert info == {"name": "HELLO.TXT", "size": 600, "start": 19}
    entry = entry_at(img, 0)
    assert entry[0] == 1 and entry[1:12] == b"HELLO   TXT"
    assert struct.unpack_from(">IIII", entry, 16) == (19, 600, T + 5, T + 5)
    # final block tail (bytes 600..1023 of the region) is zero-filled
    assert img[19 * BLOCK + 600: 21 * BLOCK] == b"\x00" * 424
    assert read_file(img, raw) == payload
    assert check(img) == []
    _, files = list_files(img)
    assert [file["name"] for file in files] == ["HELLO.TXT"]
    assert display_name(raw) == "HELLO.TXT"


def test_empty_file_claims_no_blocks():
    img = make_image()
    raw = normalize_name("EMPTY")
    put_file(img, raw, b"", T)
    start, size = struct.unpack_from(">II", entry_at(img, 0), 16)
    assert (start, size) == (0, 0)
    assert read_file(img, raw) == b""
    assert check(img) == []
    sb = read_super(img)
    assert stats(img, sb)["free_blocks"] == 2861


def test_first_fit_allocation_and_fragmentation_failure():
    img = make_image(27)  # data blocks 19..26 (8 blocks)
    a, b, c = normalize_name("A"), normalize_name("B"), normalize_name("C")
    assert put_file(img, a, b"\x01" * 1536, T)["start"] == 19  # 19..21
    assert put_file(img, b, b"\x02" * 1536, T)["start"] == 22  # 22..24
    assert put_file(img, c, b"\x03" * 1024, T)["start"] == 25  # 25..26
    delete_file(img, a)
    delete_file(img, c)
    # free = {19,20,21} + {25,26}: 5 blocks total, longest run 3
    with pytest.raises(ValueError, match="no contiguous run of 4"):
        put_file(img, normalize_name("D"), b"\x04" * 2048, T)
    assert put_file(img, normalize_name("D"), b"\x04" * 1536, T)["start"] == 19
    assert put_file(img, normalize_name("E"), b"\x05" * 512, T)["start"] == 25
    assert check(img) == []
    sb = read_super(img)
    assert stats(img, sb) == {"files": 3, "data_blocks": 8,
                              "free_blocks": 1, "leaked_blocks": 0}


def test_delete_frees_entry_before_bits_and_slot_is_reusable():
    img = make_image()
    bitmap = BITMAP_START * BLOCK
    bit19 = 0x80 >> (19 & 7)
    assert not img[bitmap + (19 >> 3)] & bit19  # fresh data block is free
    raw = normalize_name("X.TXT")
    put_file(img, raw, b"z" * 512, T)
    assert img[bitmap + (19 >> 3)] & bit19
    delete_file(img, raw)
    assert entry_at(img, 0) == b"\x00" * ENTRY_SIZE
    assert not img[bitmap + (19 >> 3)] & bit19
    assert check(img) == []
    assert stats(img, read_super(img))["free_blocks"] == 2861
    put_file(img, normalize_name("Y"), b"", T)
    assert check(img) == []


def test_replace_in_place_shrink_move_and_extend():
    img = make_image()
    bitmap = BITMAP_START * BLOCK
    f1, f2 = normalize_name("F1"), normalize_name("F2")
    put_file(img, f1, b"a" * 1024, T)  # 19,20
    put_file(img, f2, b"b" * 512, T)   # 21
    # same size: in place, start unchanged
    assert put_file(img, f1, b"c" * 1024, T + 1)["start"] == 19
    # shrink: same start, released block 20
    assert put_file(img, f1, b"d" * 512, T + 2) == {"name": "F1", "size": 512, "start": 19}
    assert not img[bitmap + (20 >> 3)] & (0x80 >> (20 & 7))
    # grow: F2 occupies 21 so the region cannot extend -> move to 22..25,
    # and the vacated block 19 must be freed (no leak)
    assert put_file(img, f1, b"e" * 2048, T + 3)["start"] == 22
    assert not img[bitmap + (19 >> 3)] & (0x80 >> (19 & 7))
    assert img[bitmap + (22 >> 3)] & (0x80 >> (22 & 7))
    entry = entry_at(img, 0)
    assert struct.unpack_from(">II", entry, 16) == (22, 2048)
    created, modified = struct.unpack_from(">II", entry, 24)
    assert (created, modified) == (T, T + 3)
    assert read_file(img, f1) == b"e" * 2048
    assert check(img) == []
    sb = read_super(img)
    assert stats(img, sb)["leaked_blocks"] == 0
    # first-fit hands the freed block 19 to the next file
    assert put_file(img, normalize_name("G"), b"g" * 512, T)["start"] == 19
    assert check(img) == []


def test_rename_keeps_fields_and_rejects_collisions():
    img = make_image()
    raw = normalize_name("hello.txt")
    put_file(img, raw, b"data", T)
    rename_file(img, raw, normalize_name("WORLD.TXT"))
    entry = entry_at(img, 0)
    assert entry[1:12] == b"WORLD   TXT"
    assert struct.unpack_from(">II", entry, 16) == (19, 4)
    assert struct.unpack_from(">II", entry, 24) == (T, T)
    assert read_file(img, normalize_name("world.txt")) == b"data"
    with pytest.raises(ValueError, match="no such file"):
        rename_file(img, raw, normalize_name("OTHER"))
    put_file(img, normalize_name("SECOND"), b"x", T)
    with pytest.raises(ValueError, match="already exists"):
        rename_file(img, normalize_name("SECOND"), normalize_name("WORLD.TXT"))
    assert check(img) == []


def test_directory_full_at_256_files():
    img = make_image()
    for number in range(DIR_ENTRIES):
        put_file(img, normalize_name(f"FILE{number:04d}"), b"", T)
    assert check(img) == []
    with pytest.raises(ValueError, match="directory is full"):
        put_file(img, normalize_name("ONEMORE"), b"", T)
    assert stats(img, read_super(img))["files"] == 256


def test_verify_rejects_boot_and_superblock_corruption():
    img = make_image()
    img[510] = 0
    assert "signature" in violations(img)

    img = make_image()
    img[BLOCK] = ord("X")
    assert "magic" in violations(img)

    img = make_image()
    struct.pack_into(">H", img, BLOCK + 4, 2)
    assert "version" in violations(img)

    img = make_image()
    struct.pack_into(">H", img, BLOCK + 6, 1)
    assert "flags" in violations(img)

    img = make_image()
    img[BLOCK + 0x24] = 0xAA
    assert "reserved" in violations(img)

    img = make_image()
    struct.pack_into(">I", img, BLOCK + 0x14, 4)  # dir_start must be 3
    assert "dir_start must be 3" in violations(img)

    img = make_image()
    struct.pack_into(">I", img, BLOCK + 0x08, 2881)
    assert "exceeds image capacity" in violations(img)

    img = make_image()
    struct.pack_into(">I", img, BLOCK + 0x08, 10)
    assert "below data_start" in violations(img)

    img = make_image()
    img.append(0)
    assert "not a multiple of 512" in violations(img)


def test_verify_rejects_entry_violations():
    img = make_image()
    put_file(img, normalize_name("A.TXT"), b"a" * 600, T)
    put_file(img, normalize_name("B.TXT"), b"b" * 512, T)
    put_file(img, normalize_name("EMPTY"), b"", T)

    bad = bytearray(img)
    patch_entry(bad, 0, 0, b"\x02")
    assert "status must be 0 or 1" in violations(bad)

    bad = bytearray(img)
    patch_entry(bad, 0, 0, b"\x00")
    assert "free entry must be all zero" in violations(bad)

    bad = bytearray(img)
    patch_entry(bad, 0, 12, b"\x01\x00\x00\x00")
    assert "reserved" in violations(bad)

    bad = bytearray(img)
    patch_entry(bad, 0, 1, b"hello   TXT")
    assert "invalid name" in violations(bad)

    bad = bytearray(img)
    patch_entry(bad, 0, 1, b"\xffLLO   TXT")
    assert "must be ASCII" in violations(bad)

    bad = bytearray(img)
    patch_entry(bad, 0, 1, b"AB CD   TXT")
    assert "invalid name" in violations(bad)

    bad = bytearray(img)
    patch_entry(bad, 1, 1, entry_at(img, 0)[1:12])
    assert "duplicate name A.TXT" in violations(bad)

    bad = bytearray(img)
    patch_entry(bad, 2, 16, struct.pack(">I", 1))
    assert "empty file must have start_block 0" in violations(bad)

    bad = bytearray(img)
    patch_entry(bad, 0, 16, struct.pack(">I", 1))  # 600-byte file below data_start
    assert "outside data area" in violations(bad)

    bad = bytearray(img)
    patch_entry(bad, 1, 16, struct.pack(">I", 19))  # B overlaps A's region
    assert "overlap" in violations(bad)


def test_verify_rejects_bitmap_violations_but_allows_leaks():
    img = make_image()
    put_file(img, normalize_name("A"), b"a" * 512, T)  # claims block 19
    bitmap = BITMAP_START * BLOCK

    bad = bytearray(img)
    bad[bitmap + (19 >> 3)] &= 0xFF ^ (0x80 >> (19 & 7))
    assert "block 19 is free in bitmap" in violations(bad)

    bad = bytearray(img)
    bad[bitmap] &= 0x7F  # metadata block 0
    assert "metadata block 0 is free in bitmap" in violations(bad)

    bad = bytearray(img)
    bad[bitmap + 360] |= 0x80  # padding bit for block 2880
    assert "bitmap padding bit for block 2880 is set" in violations(bad)

    # a set bit that no entry claims is a leak, not a violation
    img[bitmap + (50 >> 3)] |= 0x80 >> (50 & 7)
    assert check(img) == []
    assert stats(img, read_super(img))["leaked_blocks"] == 1


def test_normalize_name_rules():
    assert normalize_name("a-b_9") == b"A-B_9   " + b"   "
    assert normalize_name("x.t") == b"X       T  "
    with pytest.raises(ValueError, match="longer than 8"):
        normalize_name("toolongname.txt")
    with pytest.raises(ValueError, match="longer than 3"):
        normalize_name("file.toolong")
    with pytest.raises(ValueError, match="empty"):
        normalize_name(".txt")
    with pytest.raises(ValueError, match="longer than 3"):
        normalize_name("foo.tar.gz")  # split at the first dot: ext = "TAR.GZ"
    with pytest.raises(ValueError, match="invalid character"):
        normalize_name("a.b.c")  # ext = "B.C" contains a dot
    with pytest.raises(ValueError, match="invalid character"):
        normalize_name("bad name")


def _run(*args):
    return subprocess.run([sys.executable, str(TOOL), *args],
                          stdin=subprocess.DEVNULL, capture_output=True)


def test_cli_workflow(tmp_path):
    img = tmp_path / "disk.img"
    result = _run("mkfs", str(img), "--sectors", "2880", "--time", str(T))
    assert result.returncode == 0 and b"formatted" in result.stdout

    source = tmp_path / "hello.txt"
    payload = b"Hello, DSFS!" * 50
    source.write_bytes(payload)
    assert _run("put", str(img), str(source), "--time", str(T + 1)).returncode == 0

    result = _run("ls", str(img))
    assert result.returncode == 0 and b"HELLO.TXT" in result.stdout

    output = tmp_path / "out.bin"
    assert _run("get", str(img), "HELLO.TXT", str(output)).returncode == 0
    assert output.read_bytes() == payload

    result = _run("cat", str(img), "hello.txt")
    assert result.returncode == 0 and result.stdout == payload

    result = _run("info", str(img))
    assert result.returncode == 0
    assert b"created: 1700000000" in result.stdout and b"free_blocks" in result.stdout

    assert _run("rename", str(img), "HELLO.TXT", "WORLD.TXT").returncode == 0
    assert _run("rm", str(img), "WORLD.TXT").returncode == 0
    result = _run("verify", str(img))
    assert result.returncode == 0 and b"0 file(s)" in result.stdout

    # missing file and mismatched --sectors fail with dsfs: on stderr
    result = _run("cat", str(img), "NOPE.TXT")
    assert result.returncode == 1 and b"dsfs:" in result.stderr
    result = _run("mkfs", str(img), "--sectors", "100")
    assert result.returncode == 1 and b"dsfs:" in result.stderr
    result = _run("mkfs", str(tmp_path / "missing.img"))
    assert result.returncode == 1 and b"--sectors" in result.stderr

    # corruption makes verify exit 1 and name the violation
    data = bytearray(img.read_bytes())
    data[510] = 0
    img.write_bytes(data)
    result = _run("verify", str(img))
    assert result.returncode == 1 and b"signature" in result.stdout
