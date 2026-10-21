import pytest
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from wax import HEADER_SIZE, VERSION, is_wax, main, pack, parse


def test_roundtrip_sparse_blocks_and_debug():
    blocks = [(0x100, b"\x00\x00\x00\x00"), (0x10000, b"ABCD")]
    w = pack(0x100, 0, blocks, b'{"v":1}')
    image = parse(w)
    assert image["kind"] == "wax" and image["base"] == 0x100 and image["entry"] == 0x100
    assert image["blocks"] == [
        {"load_address": 0x100, "data": b"\x00\x00\x00\x00"},
        {"load_address": 0x10000, "data": b"ABCD"},
    ]
    assert image["debug"] == b'{"v":1}' and is_wax(w)
    assert len(w) < 0x10000 + 4 - 0x100


def test_header_fields_and_block_table_are_big_endian():
    w = pack(0x12345678, 0, [(0x12345678, b"abcd"), (0x12350000, b"efgh")])
    assert w[4:6] == VERSION.to_bytes(2, "big")
    assert w[12:16] == HEADER_SIZE.to_bytes(4, "big")
    assert w[16:20] == b"\x12\x34\x56\x78"
    assert w[24:28] == HEADER_SIZE.to_bytes(4, "big")
    assert w[28:32] == (2).to_bytes(4, "big")
    assert w[32:36] == (12).to_bytes(4, "big")
    assert w[64:76] == struct.pack(">III", 0x12345678, 88, 4)
    assert w[76:88] == struct.pack(">III", 0x12350000, 92, 4)


def test_minimal_and_wal():
    image = parse(pack(0, 0, [(0, b"\x00\x00\x00\x00")]))
    assert image["blocks"] == [{"load_address": 0, "data": b"\x00\x00\x00\x00"}]
    library = parse(pack(0, 0, [(0, b"PIC! ")], kind="wal"))
    assert library["kind"] == "wal"
    try:
        pack(0x100, 0, [(0x100, b"PIC!")], kind="wal")
    except ValueError:
        pass
    else:
        raise AssertionError("wal with nonzero base must fail")


def test_rejects_bad_magic_empty_blocks_and_old_version():
    good = pack(0, 0, [(0, b"abcd")])
    for bad in (b"\x00\x01\x02", b"BINX" + good[4:]):
        try:
            parse(bad)
        except ValueError:
            pass
        else:
            raise AssertionError("must reject")
    try:
        pack(0, 0, [])
    except ValueError:
        pass
    else:
        raise AssertionError("empty block list must fail")
    old = bytearray(good)
    old[4:6] = (1).to_bytes(2, "big")
    try:
        parse(old)
    except ValueError:
        pass
    else:
        raise AssertionError("v1 must fail")


def test_rejects_entry_in_gap_and_invalid_block_ranges():
    w = bytearray(pack(0x1000, 0, [(0x1000, b"\x00" * 4), (0x2000, b"\x00" * 4)]))
    struct.pack_into(">I", w, 20, 0x100)  # entry is in the unmapped gap
    try:
        parse(w)
    except ValueError:
        pass
    else:
        raise AssertionError("entry in gap must fail")

    overlap = bytearray(pack(0x100, 0, [(0x100, b"abcd"), (0x200, b"efgh")]))
    struct.pack_into(">I", overlap, 64 + 12, 0x102)
    try:
        parse(overlap)
    except ValueError:
        pass
    else:
        raise AssertionError("overlapping load ranges must fail")

    out_of_range = bytearray(pack(0x100, 0, [(0x100, b"abcd")]))
    struct.pack_into(">I", out_of_range, 64 + 4, 0xFFFFFFFF)
    try:
        parse(out_of_range)
    except ValueError:
        pass
    else:
        raise AssertionError("out-of-range payload must fail")


    reserved = bytearray(pack(0x100, 0, [(0x100, b"abcd")]))
    reserved[63] = 1
    try:
        parse(reserved)
    except ValueError:
        pass
    else:
        raise AssertionError("nonzero reserved bytes must fail")

def test_cli_packs_repeated_block_arguments(tmp_path):
    first = tmp_path / "first.bin"
    second = tmp_path / "second.bin"
    output = tmp_path / "program.wax"
    first.write_bytes(b"\x00\x00\x00\x00")
    second.write_bytes(b"\xfc\x00\x00\x00")
    main([
        "pack", "--base", "0x100", "--entry-offset", "0",
        "--block", f"0x100={first}", "--block", f"0x10000={second}",
        "-o", str(output),
    ])
    image = parse(output.read_bytes())
    assert [block["load_address"] for block in image["blocks"]] == [0x100, 0x10000]


def test_protected_roundtrip_describes_user_regions():
    from wax import FLAG_PROTECTED, PERM_R, PERM_W, PERM_X, user_regions

    data = pack(0x100000, 0, [(0x100000, b"\0" * 8, PERM_R | PERM_X),
                              (0x110000, b"\1" * 4, PERM_R | PERM_W)],
                protection=(0x200000, 0x1000))
    assert struct.unpack_from(">H", data, 10)[0] == FLAG_PROTECTED
    assert struct.unpack_from(">I", data, 32)[0] == 16
    assert struct.unpack_from(">II", data, 48) == (0x200000, 0x1000)
    image = parse(data)
    assert image["protection"] == (0x200000, 0x1000)
    assert [block["perm"] for block in image["blocks"]] == [5, 3]
    assert user_regions(image) == [(0x1FF000, 0x200000, 3),
                                   (0x100000, 0x100008, 5),
                                   (0x110000, 0x110004, 3)]
    assert user_regions(parse(pack(0, 0, [(0, b"\0" * 4)]))) is None


def test_protected_rejects_bad_permissions_stack_and_entry():
    from wax import PERM_R, PERM_X

    code = b"\0" * 8
    bad = [
        ([(0x1000, code, PERM_R)], (0x8000, 0x100)),          # entry not executable
        ([(0x1000, code, 0)], (0x8000, 0x100)),               # no permission
        ([(0x1000, code, 8)], (0x8000, 0x100)),               # unknown bit
        ([(0x1000, code, PERM_X)], (0x1004, 4)),              # stack overlaps code
        ([(0x1000, code, PERM_X)], (0x8002, 4)),              # misaligned
        ([(0x1000, code, PERM_X)], (0xFFFF1000, 4)),          # CPU-reserved
        ([(0x1000 + 0x100 * n, code, PERM_X) for n in range(8)], (0x8000, 0x100)),
    ]
    for blocks, protection in bad:
        with pytest.raises(ValueError):
            pack(blocks[0][0], 0, blocks, protection=protection)
    with pytest.raises(ValueError):
        pack(0, 0, [(0, code, PERM_X)], kind="wal", protection=(0x8000, 4))


def test_cli_packs_protected_blocks(tmp_path):
    from wax import main as wax_main

    code = tmp_path / "code.bin"
    code.write_bytes(b"\0" * 8)
    out = tmp_path / "app.wax"
    wax_main(["pack", "--protected", "--base", "0x1000", "--block", f"0x1000={code}:rx",
              "--stack-top", "0x8000", "--stack-size", "0x400", "-o", str(out)])
    image = parse(out.read_bytes())
    assert image["protection"] == (0x8000, 0x400)
    assert image["blocks"][0]["perm"] == 5
