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
FLAG_PROTECTED = 2  # run in Extension P user mode (doc/spec_WAX.md)
PERM_R, PERM_W, PERM_X = 1, 2, 4
MAX_PROTECTED_BLOCKS = 7  # region 0 is the user stack
CPU_RESERVED_BASE = 0xFFFF0000
_FMT = ">4sHHHHIIIIIIIII16s"


def parse_perm(text):
    """'rwx'-style permission letters -> R/W/X bit mask."""
    bits = {"r": PERM_R, "w": PERM_W, "x": PERM_X}
    mask = 0
    for letter in text.lower():
        if letter not in bits:
            raise ValueError(f"bad block permission {text!r}")
        mask |= bits[letter]
    return mask


def _validate_protection(stack_top, stack_size, blocks, entry):
    if not stack_size or stack_size % 4 or stack_top % 4:
        raise ValueError("protected stack top/size must be nonzero multiples of 4")
    if stack_top > CPU_RESERVED_BASE or stack_size > stack_top:
        raise ValueError("protected stack lies outside user-addressable memory")
    stack_start = stack_top - stack_size
    for address, payload, perm in blocks:
        if not perm or perm & ~(PERM_R | PERM_W | PERM_X):
            raise ValueError("protected block permission must be a nonzero R/W/X mask")
        if stack_start < address + len(payload) and address < stack_top:
            raise ValueError("protected stack overlaps an image block")
        if address + len(payload) > CPU_RESERVED_BASE:
            raise ValueError("protected block reaches the CPU-reserved area")
    if not any(address <= entry < address + len(payload) and perm & PERM_X
               for address, payload, perm in blocks):
        raise ValueError("protected entry must lie in an executable block")


def _align4(n):
    return (n + 3) & ~3


def pack(base, entry_offset, blocks, debug=b"", kind="wax", protection=None):
    """Blocks are (address, data) or (address, data, perm).

    `protection` = (stack_top, stack_size) makes a PROTECTED .wax; every
    block then needs an R/W/X `perm`.
    """
    if kind not in ("wax", "wal"):
        raise ValueError("kind must be 'wax' or 'wal'")
    ftype = TYPE_WAX if kind == "wax" else TYPE_WAL
    if protection is not None and kind != "wax":
        raise ValueError("PROTECTED is only valid for .wax executables")
    blocks = sorted((block[0], bytes(block[1]), block[2] if len(block) > 2 else 0)
                    for block in blocks)
    if not blocks:
        raise ValueError("at least one data block is required")
    if not 0 <= base <= 0xFFFFFFFF or not 0 <= entry_offset <= 0xFFFFFFFF:
        raise ValueError("base and entry_offset must fit u32")
    if ftype == TYPE_WAL and (base != 0 or entry_offset != 0):
        raise ValueError(".wal base/entry must be 0")

    previous_end = None
    for address, payload, _perm in blocks:
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
                                 for address, payload, _perm in blocks):
        raise ValueError("entry outside image blocks")
    entry_size = 12
    stack_fields = b"\x00" * 16
    if protection is not None:
        stack_top, stack_size = protection
        if len(blocks) > MAX_PROTECTED_BLOCKS:
            raise ValueError(f"protected WAX allows at most {MAX_PROTECTED_BLOCKS} blocks")
        _validate_protection(stack_top, stack_size, blocks, entry)
        entry_size = 16
        stack_fields = struct.pack(">II8x", stack_top, stack_size)

    debug = bytes(debug or b"")
    block_table_offset = HEADER_SIZE
    block_table_size = len(blocks) * entry_size
    cursor = block_table_offset + block_table_size
    file_blocks = []
    for address, payload, perm in blocks:
        cursor = _align4(cursor)
        file_blocks.append((address, cursor, payload, perm))
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

    flags = (FLAG_DEBUG if debug else 0) | (FLAG_PROTECTED if protection is not None else 0)
    buf = bytearray(file_size)
    buf[:HEADER_SIZE] = struct.pack(
        _FMT, MAGIC, VERSION, ftype, ARCH_SRC32, flags, HEADER_SIZE,
        base, entry_offset, block_table_offset, len(blocks), entry_size,
        debug_offset, len(debug), file_size, stack_fields,
    )
    for index, (address, file_offset, payload, perm) in enumerate(file_blocks):
        record = block_table_offset + index * entry_size
        struct.pack_into(">III", buf, record, address, file_offset, len(payload))
        if protection is not None:
            struct.pack_into(">I", buf, record + 12, perm)
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
    if flags & ~(FLAG_DEBUG | FLAG_PROTECTED):
        raise ValueError(f"bad flags {flags:#x}")
    protected = bool(flags & FLAG_PROTECTED)
    if protected and ftype != TYPE_WAX:
        raise ValueError("PROTECTED is only valid for .wax executables")
    # Protected images keep the user stack in 48..56; the rest is reserved.
    if any(data[56 if protected else 48:HEADER_SIZE]):
        raise ValueError("reserved header bytes must be 0")
    if file_size != len(data):
        raise ValueError("file_size mismatch")
    if not block_count:
        raise ValueError("empty block table")
    if block_entry_size != (16 if protected else 12):
        raise ValueError(f"bad block_entry_size {block_entry_size}")
    if protected and block_count > MAX_PROTECTED_BLOCKS:
        raise ValueError(f"protected WAX allows at most {MAX_PROTECTED_BLOCKS} blocks")
    if table_offset % 4 or table_offset < HEADER_SIZE:
        raise ValueError("block table misaligned or inside header")
    table_end = table_offset + block_count * block_entry_size
    if table_end > file_size:
        raise ValueError("block table out of range")

    blocks = []
    file_spans = [(table_offset, table_end)]
    previous_load_end = None
    for index in range(block_count):
        record = table_offset + index * block_entry_size
        address, file_offset, size = struct.unpack_from(">III", data, record)
        perm = struct.unpack_from(">I", data, record + 12)[0] if protected else 0
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
        block = {"load_address": address, "data": data[file_offset:file_end]}
        if protected:
            block["perm"] = perm  # only protected images carry permissions
        blocks.append(block)

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
    protection = None
    if protected:
        protection = struct.unpack_from(">II", data, 48)
        _validate_protection(*protection,
                             [(b["load_address"], b["data"], b["perm"]) for b in blocks],
                             entry)
    return {"kind": "wal" if is_lib else "wax", "base": base,
            "entry_offset": eoff, "entry": entry, "blocks": blocks,
            "debug": debug, "protection": protection}


def user_regions(image):
    """Extension P regions a loader programs before entering user mode:
    region 0 = stack (R/W), regions 1.. = blocks in table order."""
    if image["protection"] is None:
        return None
    stack_top, stack_size = image["protection"]
    regions = [(stack_top - stack_size, stack_top, PERM_R | PERM_W)]
    regions.extend((b["load_address"], b["load_address"] + len(b["data"]), b["perm"])
                   for b in image["blocks"])
    return regions


def _perm_text(perm):
    return "".join(letter for bit, letter in ((PERM_R, "r"), (PERM_W, "w"), (PERM_X, "x"))
                   if perm & bit)


def _describe(image):
    total = sum(len(block["data"]) for block in image["blocks"])
    lines = [f"kind={image['kind']} base=0x{image['base']:08X} "
             f"entry=0x{image['entry']:08X} blocks={len(image['blocks'])} "
             f"data={total} debug={len(image['debug'])}"]
    if image["protection"] is not None:
        stack_top, stack_size = image["protection"]
        lines.append(f"protected: user stack top=0x{stack_top:08X} size={stack_size}")
    lines.extend(
        f"  block {index}: load=0x{block['load_address']:08X} size={len(block['data'])}"
        + (f" perm={_perm_text(block['perm'])}" if image["protection"] is not None else "")
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
    p.add_argument("--block", action="append", required=True, metavar="ADDRESS=PATH[:PERM]",
                   help="add a payload block; may be repeated. PERM (r/w/x letters) "
                        "is required with --protected")
    p.add_argument("--protected", action="store_true",
                   help="start the executable in Extension P user mode")
    p.add_argument("--stack-top", default=None, help="user stack top (with --protected)")
    p.add_argument("--stack-size", default=None, help="user stack size (with --protected)")
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
            perm = 0
            if a.protected:
                path, sep, perm_text = path.rpartition(":")
                if not sep or not path:
                    ap.error("--protected blocks need ADDRESS=PATH:PERM")
                perm = parse_perm(perm_text)
            with open(path, "rb") as f:
                blocks.append((num(address), f.read(), perm))
        protection = None
        if a.protected:
            if a.stack_top is None or a.stack_size is None:
                ap.error("--protected needs --stack-top and --stack-size")
            protection = (num(a.stack_top), num(a.stack_size))
        elif a.stack_top is not None or a.stack_size is not None:
            ap.error("--stack-top/--stack-size need --protected")
        debug = b""
        if a.debug:
            with open(a.debug, "rb") as f:
                debug = f.read()
        out = pack(num(a.base), num(a.entry_offset), blocks, debug, a.kind, protection)
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
