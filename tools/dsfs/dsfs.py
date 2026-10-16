"""DSFS (Dead Simple File System) host utility (doc/spec_DSFS.md)."""
import argparse
import re
import struct
import sys
import time
from pathlib import Path

BLOCK = 512
MAGIC = b"DSFS"
VERSION = 1
BOOT_SIG = b"\x55\xaa"
BOOT_CODE_SIZE = 510
SUPER_OFFSET = BLOCK
SUPER_FMT = ">4sHHIIIIIII"
SUPER_FIELDS = ("magic", "version", "flags", "total_blocks", "bitmap_start",
                "bitmap_blocks", "dir_start", "dir_blocks", "data_start", "created")
SUPER_RESERVED = 0x24
BITMAP_START = 2
DIR_BLOCKS = 16
DIR_ENTRIES = 256
ENTRY_FMT = ">B11s4sIIII"
ENTRY_SIZE = struct.calcsize(ENTRY_FMT)
NAME_CHARS = frozenset("ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-")
NAME_RE = re.compile(r"[A-Z0-9_-]+ *")
EXT_RE = re.compile(r"[A-Z0-9_-]+ *|   ")
MAX_U32 = 0xFFFFFFFF
MIN_SECTORS = 19


def layout(total):
    """v1 on-disk positions for a volume (doc/spec_DSFS.md §2)."""
    bitmap_blocks = -(-total // 4096)
    dir_start = BITMAP_START + bitmap_blocks
    data_start = dir_start + DIR_BLOCKS
    return {"bitmap_start": BITMAP_START, "bitmap_blocks": bitmap_blocks,
            "dir_start": dir_start, "dir_blocks": DIR_BLOCKS, "data_start": data_start}


def normalize_name(text):
    """User-facing 8.3 name -> 11 on-disk bytes (uppercase, space padded)."""
    text = str(text)
    base, _, ext = text.partition(".") if "." in text else (text, "", "")
    base_u, ext_u = base.upper(), ext.upper()
    if not base_u:
        raise ValueError("file name must not be empty")
    if len(base_u) > 8:
        raise ValueError(f"name part longer than 8 characters: {base_u!r}")
    if len(ext_u) > 3:
        raise ValueError(f"extension longer than 3 characters: {ext_u!r}")
    for part, label in ((base_u, "name"), (ext_u, "extension")):
        bad = sorted(set(part) - NAME_CHARS)
        if bad:
            raise ValueError(f"invalid character(s) {''.join(bad)!r} in {label} of {text!r}")
    return (base_u.ljust(8) + ext_u.ljust(3)).encode("ascii")


def display_name(raw):
    """11 on-disk name bytes -> user-facing NAME.EXT."""
    name_field = raw[:8].decode("ascii").rstrip()
    ext_field = raw[8:].decode("ascii").rstrip()
    return f"{name_field}.{ext_field}" if ext_field else name_field


def _name_error(raw):
    """Return a violation message for 11 name bytes, or None when valid."""
    try:
        name_field = raw[:8].decode("ascii")
        ext_field = raw[8:].decode("ascii")
    except UnicodeDecodeError:
        return "name must be ASCII"
    if not NAME_RE.fullmatch(name_field):
        return "invalid name characters or padding"
    if not EXT_RE.fullmatch(ext_field):
        return "invalid extension characters or padding"
    return None


def read_super(img):
    if len(img) < 2 * BLOCK:
        raise ValueError("image is shorter than the boot and superblock sectors")
    fields = struct.unpack_from(SUPER_FMT, img, SUPER_OFFSET)
    return dict(zip(SUPER_FIELDS, fields))


def _bitmap_offset(sb, index):
    return sb["bitmap_start"] * BLOCK + (index >> 3)


def _bit_get(img, sb, index):
    return bool(img[_bitmap_offset(sb, index)] & (0x80 >> (index & 7)))


def _bit_set(img, sb, index, used):
    offset = _bitmap_offset(sb, index)
    mask = 0x80 >> (index & 7)
    img[offset] = (img[offset] | mask) if used else (img[offset] & (0xFF ^ mask))


def _bits_set(img, sb, start, end, used):
    for index in range(start, end):
        _bit_set(img, sb, index, used)


def _find_run(img, sb, count):
    """First-fit contiguous run of `count` free data blocks, or None."""
    run_start = None
    run = 0
    for index in range(sb["data_start"], sb["total_blocks"]):
        if _bit_get(img, sb, index):
            run_start, run = None, 0
            continue
        if run_start is None:
            run_start = index
        run += 1
        if run == count:
            return run_start
    return None


def _entries(img, sb):
    offset = sb["dir_start"] * BLOCK
    return [bytes(img[offset + i * ENTRY_SIZE: offset + (i + 1) * ENTRY_SIZE])
            for i in range(DIR_ENTRIES)]


def _write_entry(img, sb, index, entry):
    offset = sb["dir_start"] * BLOCK + index * ENTRY_SIZE
    img[offset:offset + ENTRY_SIZE] = bytes(entry)


def _find_entry(entries, raw_name):
    for index, entry in enumerate(entries):
        if entry[0] == 1 and entry[1:12] == raw_name:
            return index
    return None


def mkfs(img, created=None):
    """Format `img` in place (doc/spec_DSFS.md §8)."""
    if len(img) % BLOCK:
        raise ValueError(f"image length {len(img)} is not a multiple of 512")
    total = len(img) // BLOCK
    if total > MAX_U32:
        raise ValueError(f"image has {total} sectors, which exceeds u32 total_blocks")
    if total < MIN_SECTORS:
        raise ValueError(f"image too small: {total} sectors, DSFS needs at least {MIN_SECTORS}")
    lay = layout(total)
    if total < lay["data_start"]:
        raise ValueError(f"image too small: {total} sectors, data area would start at {lay['data_start']}")
    if created is None:
        created = int(time.time())
    if not 0 <= created <= MAX_U32:
        raise ValueError(f"format timestamp {created} does not fit u32")

    img[0:BOOT_CODE_SIZE] = b"\x00" * BOOT_CODE_SIZE
    img[BOOT_CODE_SIZE:BLOCK] = BOOT_SIG
    img[SUPER_OFFSET:SUPER_OFFSET + SUPER_RESERVED] = b"\x00" * SUPER_RESERVED
    struct.pack_into(SUPER_FMT, img, SUPER_OFFSET, MAGIC, VERSION, 0, total,
                     lay["bitmap_start"], lay["bitmap_blocks"], lay["dir_start"],
                     lay["dir_blocks"], lay["data_start"], created)
    img[SUPER_OFFSET + SUPER_RESERVED:SUPER_OFFSET + BLOCK] = \
        b"\x00" * (BLOCK - SUPER_RESERVED)

    bitmap_offset = BITMAP_START * BLOCK
    bitmap_length = lay["bitmap_blocks"] * BLOCK
    img[bitmap_offset:bitmap_offset + bitmap_length] = b"\x00" * bitmap_length
    full_bytes, remainder = divmod(lay["data_start"], 8)
    img[bitmap_offset:bitmap_offset + full_bytes] = b"\xff" * full_bytes
    if remainder:
        img[bitmap_offset + full_bytes] = 0xFF ^ (0xFF >> remainder)

    dir_offset = lay["dir_start"] * BLOCK
    img[dir_offset:dir_offset + DIR_BLOCKS * BLOCK] = b"\x00" * (DIR_BLOCKS * BLOCK)
    return lay


def check(img):
    """Mount validation (doc/spec_DSFS.md §7): list of violation messages."""
    violations = []
    length = len(img)
    if length % BLOCK:
        return [f"image length {length} is not a multiple of 512"]
    if length < 2 * BLOCK:
        return ["image is shorter than the boot and superblock sectors"]
    if bytes(img[BOOT_CODE_SIZE:BLOCK]) != BOOT_SIG:
        violations.append("boot signature 0x55AA missing at offset 510")

    sb = read_super(img)
    if sb["magic"] != MAGIC:
        violations.append("superblock magic is not DSFS")
        return violations
    if sb["version"] != VERSION:
        violations.append(f"unsupported superblock version {sb['version']}")
    if sb["flags"] != 0:
        violations.append(f"superblock flags must be 0, got {sb['flags']}")
    if any(img[SUPER_OFFSET + SUPER_RESERVED:SUPER_OFFSET + BLOCK]):
        violations.append("superblock reserved bytes (0x024..0x1FF) must be zero")

    total = sb["total_blocks"]
    capacity = length // BLOCK
    expected = layout(total)
    for key in ("bitmap_start", "bitmap_blocks", "dir_start", "dir_blocks", "data_start"):
        if sb[key] != expected[key]:
            violations.append(f"superblock {key} must be {expected[key]}, got {sb[key]}")
    if total < expected["data_start"]:
        violations.append(f"total_blocks {total} is below data_start {expected['data_start']}")
    if total > capacity:
        violations.append(f"total_blocks {total} exceeds image capacity {capacity}")
    if violations:
        return violations

    data_start = expected["data_start"]
    seen = {}
    regions = []
    for index, entry in enumerate(_entries(img, sb)):
        status = entry[0]
        if status == 0:
            if any(entry):
                violations.append(f"entry {index}: free entry must be all zero")
            continue
        if status != 1:
            violations.append(f"entry {index}: status must be 0 or 1, got {status}")
            continue
        if any(entry[12:16]):
            violations.append(f"entry {index}: reserved bytes must be zero")
        raw_name = entry[1:12]
        name_violation = _name_error(raw_name)
        if name_violation:
            violations.append(f"entry {index}: {name_violation}")
        elif raw_name in seen:
            violations.append(
                f"duplicate name {display_name(raw_name)} in entries {seen[raw_name]} and {index}")
        else:
            seen[raw_name] = index
        start, size = struct.unpack_from(">II", entry, 16)
        if size == 0:
            if start != 0:
                violations.append(f"entry {index}: empty file must have start_block 0")
            continue
        blocks = -(-size // BLOCK)
        if start < data_start or start + blocks > total:
            violations.append(
                f"entry {index}: data range [{start}, {start + blocks}) outside data area "
                f"[{data_start}, {total})")
            continue
        regions.append((index, start, blocks))

    run_end, run_owner = -1, None
    for index, start, blocks in sorted(regions, key=lambda region: region[1]):
        if start < run_end:
            violations.append(f"entries {run_owner} and {index} overlap at block {start}")
        if start + blocks > run_end:
            run_end, run_owner = start + blocks, index
        for block in range(start, start + blocks):
            if not _bit_get(img, sb, block):
                violations.append(f"entry {index}: block {block} is free in bitmap")
                break

    for block in range(data_start):
        if not _bit_get(img, sb, block):
            violations.append(f"metadata block {block} is free in bitmap")
            break
    for block in range(total, sb["bitmap_blocks"] * 4096):
        if _bit_get(img, sb, block):
            violations.append(f"bitmap padding bit for block {block} is set")
            break
    return violations


def mount(img):
    """Validate and return the superblock, or raise ValueError."""
    violations = check(img)
    if violations:
        raise ValueError("not a mountable DSFS image: " + "; ".join(violations))
    return read_super(img)


def stats(img, sb):
    """Space usage of a mounted image: files, free and leaked data blocks."""
    bitmap_offset = sb["bitmap_start"] * BLOCK
    bitmap = bytes(img[bitmap_offset:bitmap_offset + sb["bitmap_blocks"] * BLOCK])
    used = 0
    files = 0
    for entry in _entries(img, sb):
        if entry[0] != 1:
            continue
        files += 1
        size = struct.unpack_from(">I", entry, 20)[0]
        if size:
            used += -(-size // BLOCK)
    data_ones = sum(byte.bit_count() for byte in bitmap) - sb["data_start"]
    data_blocks = sb["total_blocks"] - sb["data_start"]
    return {"files": files, "data_blocks": data_blocks,
            "free_blocks": data_blocks - data_ones, "leaked_blocks": data_ones - used}


def list_files(img):
    """Return (superblock, [file dicts]) for every used entry."""
    sb = mount(img)
    files = []
    for index, entry in enumerate(_entries(img, sb)):
        if entry[0] != 1:
            continue
        start, size, created, modified = struct.unpack_from(">IIII", entry, 16)
        files.append({"index": index, "name": display_name(entry[1:12]),
                      "size": size, "start": start,
                      "created": created, "modified": modified})
    return sb, files


def put_file(img, raw_name, data, now):
    """Create or replace a file (doc/spec_DSFS.md §8 update order)."""
    if len(raw_name) != 11:
        raise ValueError("raw name must be exactly 11 bytes")
    name_violation = _name_error(raw_name)
    if name_violation:
        raise ValueError(name_violation)
    data = bytes(data)
    if len(data) > MAX_U32:
        raise ValueError("file content exceeds u32 size_bytes (4 GiB - 1)")
    if not 0 <= now <= MAX_U32:
        raise ValueError(f"timestamp {now} does not fit u32")

    sb = mount(img)
    entries = [bytearray(entry) for entry in _entries(img, sb)]
    index = _find_entry(entries, raw_name)
    existing = index is not None
    if not existing:
        index = next((i for i, entry in enumerate(entries) if entry[0] == 0), None)
        if index is None:
            raise ValueError("directory is full (256 files)")
    old_start, old_size = (0, 0)
    created = now
    if existing:
        old_start, old_size, created, _ = struct.unpack_from(">IIII", entries[index], 16)
    old_blocks = -(-old_size // BLOCK) if old_size else 0

    needed = -(-len(data) // BLOCK)
    if needed == 0:
        new_start, new_claimed = 0, 0
    elif existing and old_blocks and needed <= old_blocks:
        new_start, new_claimed = old_start, needed
    elif existing and old_blocks and old_start + needed <= sb["total_blocks"] and \
            all(not _bit_get(img, sb, block)
                for block in range(old_start + old_blocks, old_start + needed)):
        _bits_set(img, sb, old_start + old_blocks, old_start + needed, True)
        new_start, new_claimed = old_start, needed
    else:
        run = _find_run(img, sb, needed)
        if run is None:
            free_blocks = stats(img, sb)["free_blocks"]
            raise ValueError(
                f"no contiguous run of {needed} block(s) (fragmented free space, "
                f"{free_blocks} block(s) free)")
        _bits_set(img, sb, run, run + needed, True)
        new_start, new_claimed = run, needed

    if new_claimed:
        offset = new_start * BLOCK
        img[offset:offset + len(data)] = data
        tail = new_claimed * BLOCK - len(data)
        img[offset + len(data):offset + new_claimed * BLOCK] = b"\x00" * tail

    struct.pack_into(">B11s4sIIII", entries[index], 0, 1, raw_name,
                     b"\x00\x00\x00\x00", new_start, len(data), created, now)
    if existing and old_blocks:
        for block in range(old_start, old_start + old_blocks):
            if not (new_start <= block < new_start + new_claimed):
                _bit_set(img, sb, block, False)
    _write_entry(img, sb, index, entries[index])
    return {"name": display_name(raw_name), "size": len(data), "start": new_start}


def read_file(img, raw_name):
    sb = mount(img)
    entries = _entries(img, sb)
    index = _find_entry(entries, raw_name)
    if index is None:
        raise ValueError(f"no such file: {display_name(raw_name)}")
    start, size = struct.unpack_from(">II", entries[index], 16)
    return bytes(img[start * BLOCK:start * BLOCK + size])


def delete_file(img, raw_name):
    sb = mount(img)
    entries = _entries(img, sb)
    index = _find_entry(entries, raw_name)
    if index is None:
        raise ValueError(f"no such file: {display_name(raw_name)}")
    start, size = struct.unpack_from(">II", entries[index], 16)
    blocks = -(-size // BLOCK) if size else 0
    _write_entry(img, sb, index, b"\x00" * ENTRY_SIZE)
    if blocks:
        _bits_set(img, sb, start, start + blocks, False)


def rename_file(img, raw_old, raw_new):
    if len(raw_new) != 11:
        raise ValueError("raw name must be exactly 11 bytes")
    name_violation = _name_error(raw_new)
    if name_violation:
        raise ValueError(name_violation)
    sb = mount(img)
    entries = [bytearray(entry) for entry in _entries(img, sb)]
    index = _find_entry(entries, raw_old)
    if index is None:
        raise ValueError(f"no such file: {display_name(raw_old)}")
    other = _find_entry(entries, raw_new)
    if other is not None and other != index:
        raise ValueError(f"name already exists: {display_name(raw_new)}")
    entries[index][1:12] = raw_new
    _write_entry(img, sb, index, entries[index])


def _load(path):
    return bytearray(Path(path).read_bytes())


def _save(path, img):
    Path(path).write_bytes(bytes(img))


def _dispatch(args):
    if args.cmd == "mkfs":
        path = Path(args.image)
        if path.exists():
            img = _load(path)
            if args.sectors is not None and len(img) != args.sectors * BLOCK:
                raise ValueError(
                    f"image exists with {len(img)} bytes but --sectors {args.sectors} given")
        else:
            if args.sectors is None:
                raise ValueError("--sectors is required to create a new image")
            if args.sectors <= 0:
                raise ValueError("--sectors must be positive")
            if args.sectors > MAX_U32:
                raise ValueError("--sectors exceeds u32 total_blocks")
            img = bytearray(args.sectors * BLOCK)
        created = args.ts if args.ts is not None else int(time.time())
        mkfs(img, created)
        _save(path, img)
        sb = read_super(img)
        print(f"formatted {path}: {sb['total_blocks']} sectors, "
              f"{sb['total_blocks'] - sb['data_start']} data blocks")
        return 0

    img = _load(args.image)
    if args.cmd == "verify":
        violations = check(img)
        if violations:
            for line in violations:
                print(line)
            print(f"{len(violations)} violation(s)")
            return 1
        sb = read_super(img)
        st = stats(img, sb)
        print(f"OK: {st['files']} file(s), {st['free_blocks']} free block(s), "
              f"{st['leaked_blocks']} leaked block(s)")
        return 0

    if args.cmd == "info":
        sb = mount(img)
        st = stats(img, sb)
        print(f"total_blocks: {sb['total_blocks']}")
        print(f"bitmap_start: {sb['bitmap_start']} ({sb['bitmap_blocks']} block(s))")
        print(f"dir_start: {sb['dir_start']} ({sb['dir_blocks']} block(s), "
              f"{DIR_ENTRIES} entries)")
        print(f"data_start: {sb['data_start']} ({st['data_blocks']} data block(s))")
        print(f"created: {sb['created']}")
        print(f"files: {st['files']}")
        print(f"free_blocks: {st['free_blocks']}")
        print(f"leaked_blocks: {st['leaked_blocks']}")
        return 0

    if args.cmd == "ls":
        sb, files = list_files(img)
        st = stats(img, sb)
        print(f"{'NAME':<12} {'SIZE':>10} {'START':>6} {'CREATED':>10} {'MODIFIED':>10}")
        for file in files:
            print(f"{file['name']:<12} {file['size']:>10} {file['start']:>6} "
                  f"{file['created']:>10} {file['modified']:>10}")
        print(f"{len(files)} file(s), {st['free_blocks']} free block(s), "
              f"{st['leaked_blocks']} leaked block(s)")
        return 0

    if args.cmd == "put":
        raw_name = normalize_name(
            args.name if args.name is not None else Path(args.file).name)
        data = Path(args.file).read_bytes()
        now = args.ts if args.ts is not None else int(time.time())
        put_file(img, raw_name, data, now)
        _save(args.image, img)
        return 0

    if args.cmd == "rename":
        rename_file(img, normalize_name(args.old), normalize_name(args.new_name))
        _save(args.image, img)
        return 0

    raw_name = normalize_name(args.name)
    if args.cmd == "get":
        Path(args.output).write_bytes(read_file(img, raw_name))
        return 0
    if args.cmd == "cat":
        sys.stdout.buffer.write(read_file(img, raw_name))
        return 0
    if args.cmd == "rm":
        delete_file(img, raw_name)
        _save(args.image, img)
        return 0
    raise AssertionError(f"unknown command {args.cmd}")


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="dsfs.py", description="DSFS host utility (doc/spec_DSFS.md)")
    sub = parser.add_subparsers(dest="cmd", required=True)
    int0 = lambda text: int(text, 0)

    p = sub.add_parser("mkfs", help="format an existing image or create a new one")
    p.add_argument("image")
    p.add_argument("--sectors", type=int0, metavar="N",
                   help="sector count; required for a new image")
    p.add_argument("--time", dest="ts", type=int0, metavar="UNIX",
                   help="format timestamp (default: now)")

    sub.add_parser("info", help="show superblock and space usage").add_argument("image")
    sub.add_parser("ls", help="list files").add_argument("image")

    p = sub.add_parser("put", help="create or replace a file")
    p.add_argument("image")
    p.add_argument("file")
    p.add_argument("name", nargs="?", help="8.3 name (default: source file name)")
    p.add_argument("--time", dest="ts", type=int0, metavar="UNIX",
                   help="timestamp (default: now)")

    p = sub.add_parser("get", help="extract a file")
    p.add_argument("image")
    p.add_argument("name")
    p.add_argument("output")

    p = sub.add_parser("cat", help="write a file to stdout")
    p.add_argument("image")
    p.add_argument("name")

    p = sub.add_parser("rm", help="delete a file")
    p.add_argument("image")
    p.add_argument("name")

    p = sub.add_parser("rename", help="rename a file")
    p.add_argument("image")
    p.add_argument("old")
    p.add_argument("new_name")

    sub.add_parser("verify", help="run mount validation (§7)").add_argument("image")

    args = parser.parse_args(argv)
    try:
        return _dispatch(args)
    except (ValueError, OSError) as error:
        print(f"dsfs: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
