"""WAX container pack/parse (spec doc/spec_WAX.md v1)."""
import argparse
import struct

MAGIC = b"WAX\x00"
VERSION = 1
ARCH_SRC32 = 1
TYPE_WAX = 1
TYPE_WAL = 2
HEADER_SIZE = 64
FLAG_DEBUG = 1
_FMT = "<4sHHHHIIIIIIIII16s"


def _align4(n):
    return (n + 3) & ~3


def pack(base, entry_offset, text, rodata=b"", debug=b"", kind="wax"):
    if not text:
        raise ValueError("text must not be empty")
    ftype = TYPE_WAX if kind == "wax" else TYPE_WAL
    if ftype == TYPE_WAL and (base != 0 or entry_offset != 0):
        raise ValueError(".wal base/entry must be 0")
    flags = FLAG_DEBUG if debug else 0
    blobs = [bytes(text), bytes(rodata or b""), bytes(debug or b"")]
    offs, out = [], HEADER_SIZE
    for b in blobs:
        if b:
            offs.append(out)
            out = _align4(out + len(b))
        else:
            offs.append(0)
    buf = bytearray(out)
    hdr = struct.pack(
        _FMT, MAGIC, VERSION, ftype, ARCH_SRC32, flags, HEADER_SIZE,
        base & 0xFFFFFFFF, entry_offset & 0xFFFFFFFF,
        offs[0], len(blobs[0]), offs[1] if blobs[1] else 0, len(blobs[1]),
        offs[2] if blobs[2] else 0, len(blobs[2]), b"\x00" * 16)
    buf[:HEADER_SIZE] = hdr
    for o, b in zip(offs, blobs):
        if b:
            buf[o:o + len(b)] = b
    if kind == "wax":
        p = parse(bytes(buf))  # self-check
        if p["entry"] != (base + entry_offset) & 0xFFFFFFFF:
            raise AssertionError("entry self-check failed")
    return bytes(buf)


def parse(data):
    data = bytes(data)
    if len(data) < HEADER_SIZE:
        raise ValueError("too small for WAX header")
    if data[:4] != MAGIC:
        raise ValueError("bad magic (not WAX)")
    (magic, ver, ftype, arch, flags, hsz, base, eoff,
     toff, tsz, roff, rsz, doff, dsz, _res) = struct.unpack(_FMT, data[:HEADER_SIZE])
    if ver != VERSION:
        raise ValueError(f"unsupported version {ver}")
    if ftype not in (TYPE_WAX, TYPE_WAL):
        raise ValueError(f"bad type {ftype}")
    if arch != ARCH_SRC32:
        raise ValueError(f"bad arch {arch}")
    if hsz != HEADER_SIZE:
        raise ValueError(f"bad header_size {hsz}")
    if flags & ~FLAG_DEBUG:
        raise ValueError(f"bad flags {flags:#x}")
    if not tsz:
        raise ValueError("empty text")
    for name, o, s in (("text", toff, tsz), ("rodata", roff, rsz), ("debug", doff, dsz)):
        if not s:
            continue
        if o % 4:
            raise ValueError(f"{name} misaligned")
        if o + s > len(data) or o < HEADER_SIZE:
            raise ValueError(f"{name} out of range")
    spans = sorted((o, o + s) for _, o, s in
                   (("text", toff, tsz), ("rodata", roff, rsz), ("debug", doff, dsz)) if s)
    for (a0, a1), (b0, b1) in zip(spans, spans[1:]):
        if b0 < a1:
            raise ValueError("overlapping blobs")
    if bool(dsz) != bool(flags & FLAG_DEBUG):
        raise ValueError("debug flag/size mismatch")
    if ftype == TYPE_WAL and (base != 0 or eoff != 0):
        raise ValueError(".wal base/entry must be 0")
    entry = (base + eoff) & 0xFFFFFFFF
    if ftype == TYPE_WAX and not (base <= entry < base + tsz + rsz):
        # entry must land in loaded image (text+rodata extent)
        if not (base <= entry < (base + tsz) & 0xFFFFFFFF or
                (base + tsz) & 0xFFFFFFFF <= entry < (base + tsz + rsz) & 0xFFFFFFFF):
            raise ValueError("entry outside image")
    if (base + tsz + rsz) & 0xFFFFFFFF < base and tsz + rsz:
        raise ValueError("address wrap")
    return {"kind": "wax" if ftype == TYPE_WAX else "wal", "base": base,
            "entry_offset": eoff, "entry": entry,
            "text": data[toff:toff + tsz], "rodata": data[roff:roff + rsz] if rsz else b"",
            "debug": data[doff:doff + dsz] if dsz else b""}


def is_wax(data):
    return bytes(data)[:4] == MAGIC


def main(argv=None):
    ap = argparse.ArgumentParser(description="WAX pack/unpack")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("pack")
    p.add_argument("--kind", default="wax", choices=["wax", "wal"])
    p.add_argument("--base", default="0")
    p.add_argument("--entry-offset", default="0")
    p.add_argument("--text", required=True)
    p.add_argument("--rodata", default=None)
    p.add_argument("--debug", default=None)
    p.add_argument("-o", "--output", required=True)
    u = sub.add_parser("unpack")
    u.add_argument("input")
    u.add_argument("--outdir", default=".")
    sub.add_parser("info").add_argument("input")
    a = ap.parse_args(argv)
    num = lambda s: int(s, 0)
    if a.cmd == "pack":
        import os
        with open(a.text, "rb") as f:
            text = f.read()
        rd = open(a.rodata, "rb").read() if a.rodata else b""
        dg = open(a.debug, "rb").read() if a.debug else b""
        out = pack(num(a.base), num(a.entry_offset), text, rd, dg, a.kind)
        with open(a.output, "wb") as f:
            f.write(out)
        print(f"wrote {a.output} ({len(out)} bytes)")
    elif a.cmd == "unpack":
        import os
        with open(a.input, "rb") as f:
            p_ = parse(f.read())
        os.makedirs(a.outdir, exist_ok=True)
        for n in ("text", "rodata", "debug"):
            if p_[n]:
                open(os.path.join(a.outdir, n + ".bin"), "wb").write(p_[n])
        print(f"kind={p_['kind']} base=0x{p_['base']:08X} entry=0x{p_['entry']:08X} "
              f"text={len(p_['text'])} rodata={len(p_['rodata'])} debug={len(p_['debug'])}")
    else:
        with open(a.input, "rb") as f:
            p_ = parse(f.read())
        print(f"kind={p_['kind']} base=0x{p_['base']:08X} entry=0x{p_['entry']:08X} "
              f"text={len(p_['text'])} rodata={len(p_['rodata'])} debug={len(p_['debug'])}")


if __name__ == "__main__":
    main()
