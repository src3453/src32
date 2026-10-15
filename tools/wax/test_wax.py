import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(__file__))
from wax import is_wax, pack, parse


def test_roundtrip():
    w = pack(0x100, 0, b"\x00\x00\x00\x00", b"ABCD", b'{"v":1}')
    p = parse(w)
    assert p["kind"] == "wax" and p["base"] == 0x100 and p["entry"] == 0x100
    assert p["text"] == b"\x00\x00\x00\x00" and p["rodata"] == b"ABCD"
    assert p["debug"] == b'{"v":1}' and is_wax(w)


def test_minimal():
    p = parse(pack(0, 0, b"\x00\x00\x00\x00"))
    assert (p["base"], p["entry"], p["rodata"], p["debug"]) == (0, 0, b"", b"")


def test_wal():
    p = parse(pack(0, 0, b"\x00\x00\x00\x00", kind="wal"))
    assert p["kind"] == "wal"
    try:
        pack(0x100, 0, b"\x00\x00\x00\x00", kind="wal")
    except ValueError:
        pass
    else:
        raise AssertionError("wal with base must fail")


def test_rejects():
    good = pack(0, 0, b"\x00\x00\x00\x00")
    for bad in (b"\x00\x01\x02", b"BINX" + good[4:]):
        try:
            parse(bad)
        except ValueError:
            pass
        else:
            raise AssertionError("must reject")
    try:
        pack(0, 0, b"")
    except ValueError:
        pass
    else:
        raise AssertionError("empty text must fail")


def test_entry_outside_rejected():
    import struct
    w = bytearray(pack(0x1000, 0, b"\x00\x00\x00\x00" * 4))
    struct.pack_into("<I", w, 20, 0x100)  # entry_offset past image
    try:
        parse(bytes(w))
    except ValueError:
        pass
    else:
        raise AssertionError("entry outside must fail")
