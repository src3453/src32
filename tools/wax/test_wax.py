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
