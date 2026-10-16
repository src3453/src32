"""WAX v2 container pack/parse (doc/spec_WAX.md)."""
import argparse
import struct

MAGIC = b"WAX\x00"
VERSION = 2
ARCH_SRC32 = 1
TYPE_WAX = 1
TYPE_WAL = 2
HEADER_SIZE = 64
FLAG_DEBUG = 1
_FMT = ">4sHHHHIIIIIIIII16s"


def _align4(n):
    return (n + 3) & ~3


def pack(base, entry_offset, blocks, debug=b"", kind="wax"):
    if kind not in ("wax", "wal"):
        raise ValueError("kind must be 'wax' or 'wal'")
    ftype = TYPE_WAX if kind == "wax" else TYPE_WAL
    blocks = sorted((address, bytes(data)) for address, data in blocks)
    if not blocks:
        raise ValueError("at least one data block is required")
    if not 0 <= base <= 0xFFFFFFFF or not 0 <= entry_offset <= 0xFFFFFFFF:
        raise ValueError("base and entry_offset must fit u32")
    if ftype == TYPE_WAL and (base != 0 or entry_offset != 0):
        raise ValueError(".wal base/entry must be 0")

    previous_end = None
    for address, payload in blocks:
        if not 0 <= address <= 0xFFFFFFFF or not payload or len(payload) > 0xFFFFFFFF:
            raise ValueError("block address and nonempty size must fit u32")
        end = address + len(payload)
        if end > 0x1_0000_0000:
            raise ValueError("block address range wraps u32")
        if previous_end is not None and address < previous_end:
            raise ValueError("overlapping blocks")
        previous_end = end
    if kind == "wax" and base != blocks[0][0]:
        raise ValueError("base must equal the lowest WAX block address")

    entry = base + entry_offset
    if entry > 0xFFFFFFFF:
        raise ValueError("entry address wraps u32")
    if kind == "wax" and not any(address <= entry < address + len(payload)
                                 for address, payload in blocks):
        raise ValueError("entry outside image blocks")

    debug = bytes(debug or b"")
    block_table_offset = HEADER_SIZE
    block_table_size = len(blocks) * 12
    cursor = block_table_offset + block_table_size
    file_blocks = []
    for address, payload in blocks:
        cursor = _align4(cursor)
        file_blocks.append((address, cursor, payload))
        cursor += len(payload)
    if debug:
        cursor = _align4(cursor)
        debug_offset = cursor
        cursor += len(debug)
    else:
        debug_offset = 0
    file_size = cursor
    if file_size > 0xFFFFFFFF or block_table_size > 0xFFFFFFFF:
        raise ValueError("WAX file is too large")

    flags = FLAG_DEBUG if debug else 0
    buf = bytearray(file_size)
    buf[:HEADER_SIZE] = struct.pack(
        _FMT, MAGIC, VERSION, ftype, ARCH_SRC32, flags, HEADER_SIZE,
        base, entry_offset, block_table_offset, len(blocks), 12,
        debug_offset, len(debug), file_size, b"\x00" * 16,
    )
    for index, (address, file_offset, payload) in enumerate(file_blocks):
        struct.pack_into(">III", buf, block_table_offset + index * 12,
                         address, file_offset, len(payload))
        buf[file_offset:file_offset + len(payload)] = payload
    if debug:
        buf[debug_offset:debug_offset + len(debug)] = debug
    return bytes(buf)


def parse(data):
    data = bytes(data)
    if len(data) < HEADER_SIZE:
        raise ValueError("too small for WAX header")
    if data[:4] != MAGIC:
        raise ValueError("bad magic (not WAX)")
    (magic, ver, ftype, arch, flags, hsz, base, eoff, table_offset,
     block_count, block_entry_size, debug_offset, debug_size, file_size,
     _res) = struct.unpack(_FMT, data[:HEADER_SIZE])
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
    if any(data[48:HEADER_SIZE]):
        raise ValueError("reserved header bytes must be 0")
    if file_size != len(data):
        raise ValueError("file_size mismatch")
    if not block_count:
        raise ValueError("empty block table")
    if block_entry_size != 12:
        raise ValueError(f"bad block_entry_size {block_entry_size}")
    if table_offset % 4 or table_offset < HEADER_SIZE:
        raise ValueError("block table misaligned or inside header")
    table_end = table_offset + block_count * block_entry_size
    if table_end > file_size:
        raise ValueError("block table out of range")

    blocks = []
    file_spans = [(table_offset, table_end)]
    previous_load_end = None
    for index in range(block_count):
        address, file_offset, size = struct.unpack_from(">III", data,
                                                         table_offset + index * block_entry_size)
        if not size:
            raise ValueError("empty data block")
        if address + size > 0x1_0000_0000:
            raise ValueError("block address range wraps u32")
        if previous_load_end is not None and address < previous_load_end:
            raise ValueError("blocks overlap or are not address-sorted")
        previous_load_end = address + size
        if file_offset % 4:
            raise ValueError("block payload misaligned")
        file_end = file_offset + size
        if file_offset < table_end or file_end > file_size:
            raise ValueError("block payload out of range")
        file_spans.append((file_offset, file_end))
        blocks.append({"load_address": address,
                       "data": data[file_offset:file_end]})

    if bool(debug_size) != bool(flags & FLAG_DEBUG):
        raise ValueError("debug flag/size mismatch")
    if debug_size:
        if debug_offset % 4 or debug_offset < table_end:
            raise ValueError("debug block misaligned or overlaps metadata")
        debug_end = debug_offset + debug_size
        if debug_end > file_size:
            raise ValueError("debug block out of range")
        file_spans.append((debug_offset, debug_end))
        debug = data[debug_offset:debug_end]
    else:
        if debug_offset:
            raise ValueError("debug_offset must be 0 when debug is absent")
        debug = b""
    file_spans.sort()
    for (_, previous_end), (current_start, _) in zip(file_spans, file_spans[1:]):
        if current_start < previous_end:
            raise ValueError("overlapping file blocks")

    is_lib = ftype == TYPE_WAL
    if is_lib and (base != 0 or eoff != 0):
        raise ValueError(".wal base/entry must be 0")
    if not is_lib and base != blocks[0]["load_address"]:
        raise ValueError("base must equal the lowest WAX block address")
    entry = base + eoff
    if entry > 0xFFFFFFFF:
        raise ValueError("entry address wraps u32")
    if not is_lib and not any(block["load_address"] <= entry <
                              block["load_address"] + len(block["data"])
                              for block in blocks):
        raise ValueError("entry outside image blocks")
    return {"kind": "wal" if is_lib else "wax", "base": base,
            "entry_offset": eoff, "entry": entry, "blocks": blocks,
            "debug": debug}


def _describe(image):
    total = sum(len(block["data"]) for block in image["blocks"])
    lines = [f"kind={image['kind']} base=0x{image['base']:08X} "
             f"entry=0x{image['entry']:08X} blocks={len(image['blocks'])} "
             f"data={total} debug={len(image['debug'])}"]
    lines.extend(
        f"  block {index}: load=0x{block['load_address']:08X} size={len(block['data'])}"
        for index, block in enumerate(image["blocks"])
    )
    return "\n".join(lines)




def is_wax(data):
    return bytes(data)[:4] == MAGIC


def main(argv=None):
    ap = argparse.ArgumentParser(description="WAX block-container pack/unpack")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("pack")
    p.add_argument("--kind", default="wax", choices=["wax", "wal"])
    p.add_argument("--base", default="0")
    p.add_argument("--entry-offset", default="0")
    p.add_argument("--block", action="append", required=True, metavar="ADDRESS=PATH",
                   help="add a payload block; may be repeated")
    p.add_argument("--debug", default=None)
    p.add_argument("-o", "--output", required=True)
    u = sub.add_parser("unpack")
    u.add_argument("input")
    u.add_argument("--outdir", default=".")
    sub.add_parser("info").add_argument("input")
    a = ap.parse_args(argv)
    num = lambda s: int(s, 0)
    if a.cmd == "pack":
        blocks = []
        for item in a.block:
            address, path = item.split("=", 1)
            with open(path, "rb") as f:
                blocks.append((num(address), f.read()))
        debug = b""
        if a.debug:
            with open(a.debug, "rb") as f:
                debug = f.read()
        out = pack(num(a.base), num(a.entry_offset), blocks, debug, a.kind)
        with open(a.output, "wb") as f:
            f.write(out)
        print(f"wrote {a.output} ({len(out)} bytes)")
    elif a.cmd == "unpack":
        import os
        with open(a.input, "rb") as f:
            image = parse(f.read())
        os.makedirs(a.outdir, exist_ok=True)
        for index, block in enumerate(image["blocks"]):
            name = f"block-{index:03d}-{block['load_address']:08X}.bin"
            with open(os.path.join(a.outdir, name), "wb") as f:
                f.write(block["data"])
        if image["debug"]:
            with open(os.path.join(a.outdir, "debug.bin"), "wb") as f:
                f.write(image["debug"])
        print(_describe(image))
    else:
        with open(a.input, "rb") as f:
            image = parse(f.read())
        print(_describe(image))


if __name__ == "__main__":
    main()
