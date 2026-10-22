"""FS2 (File System 2) host utility (doc/spec_FS2.md).

The tool works on a raw image in memory and writes it back only after an
operation succeeds, so a failed command leaves the file unchanged. It acts
as UID 0 (§8: mode checks are skipped, structural limits are not).
"""
import argparse
import os
import struct
import sys
import time
from pathlib import Path

BLOCK = 512
PAYLOAD = BLOCK - 4
MAGIC = b"FS2\0"
VERSION = 1
FLAG_DIRTY = 1
BOOT_SIG = b"\x55\xaa"
BOOT_CODE_SIZE = 510
BOOT_START = 2
ROOT_INODE = 1
CASEFOLD_VERSION = 0x00100000  # Unicode 16.0.0
INODE_SIZE = 128
ENTRY_SIZE = 1040
NAME_BYTES = 1024
NAME_CHARS = 256
FREE, FILE, DIR = 0, 1, 2
MODE_FILE = 0x0034  # owner RW, other R
MODE_DIR = 0x003D   # owner RWX, other RX
MODE_MASK = 0x003F
MAX_U32 = 0xFFFFFFFF
INVALID_UID = MAX_U32

SUPER_FMT = ">4sHHIIIIIIIIIIIIqH"
SUPER_FIELDS = ("magic", "version", "flags", "total_blocks", "boot_start", "boot_blocks", "boot_size",
                "bitmap_start", "bitmap_blocks", "inode_start", "inode_blocks", "inode_count",
                "data_start", "root_inode", "casefold_version", "created_sec", "created_ms")
SUPER_USED = struct.calcsize(SUPER_FMT)  # 0x42; the rest of the block is reserved
INODE_FMT = ">BBHIIIQIIqH2xqH2xqH"
INODE_FIELDS = ("type", "flags", "mode", "uid", "parent", "first_block", "size", "block_count",
                "reserved", "created_sec", "created_ms", "modified_sec", "modified_ms",
                "changed_sec", "changed_ms")
INODE_USED = struct.calcsize(INODE_FMT)  # 0x42; 62 reserved bytes follow
ENTRY_HEAD = struct.Struct(">IHH")


class Fs2Error(Exception):
    """An FS2 operation failed. `code` is the semantic name from §9/§10."""

    def __init__(self, code, message):
        super().__init__(f"{code}: {message}")
        self.code = code


def _load_casefold():
    table = {}
    path = Path(__file__).with_name("CaseFolding-16.0.0-CF.txt")
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        code, status, mapping, _ = (part.strip() for part in line.split(";"))
        if status in ("C", "F"):
            table[int(code, 16)] = "".join(chr(int(c, 16)) for c in mapping.split())
    return table


CASEFOLD = _load_casefold()


def fold(name):
    """Comparison key (§6): full case folding with Unicode 16.0.0 C + F."""
    return "".join(CASEFOLD.get(ord(c), c) for c in name)


def check_name(name):
    """Returns the UTF-8 bytes of a valid name or raises (§6)."""
    if name in ("", ".", ".."):
        raise Fs2Error("BAD_PATH", f"invalid name {name!r}")
    for c in name:
        o = ord(c)
        if c == "/" or o <= 0x1F or 0x7F <= o <= 0x9F or 0xD800 <= o <= 0xDFFF:
            raise Fs2Error("BAD_PATH", f"invalid character U+{o:04X} in {name!r}")
    raw = name.encode("utf-8")
    if len(name) > NAME_CHARS or len(raw) > NAME_BYTES:
        raise Fs2Error("NAME_TOO_LONG", f"name longer than {NAME_CHARS} characters / {NAME_BYTES} bytes")
    return raw


def layout(total_blocks, boot_blocks, inode_count):
    """Block positions from §2."""
    if not 1 <= inode_count <= 0xFFFFFFFE:
        raise Fs2Error("FORMAT", "inode_count must be 1..0xFFFFFFFE")
    bitmap_start = BOOT_START + boot_blocks
    bitmap_blocks = -(-total_blocks // 4096)
    inode_start = bitmap_start + bitmap_blocks
    inode_blocks = -(-inode_count // 4)
    data_start = inode_start + inode_blocks
    return {"boot_start": BOOT_START, "bitmap_start": bitmap_start, "bitmap_blocks": bitmap_blocks,
            "inode_start": inode_start, "inode_blocks": inode_blocks, "data_start": data_start}


def now_time():
    """(sec, ms) now, or SOURCE_DATE_EPOCH (whole seconds) for reproducible images."""
    epoch = os.environ.get("SOURCE_DATE_EPOCH")
    if epoch is not None:
        return int(epoch), 0
    ns = time.time_ns()
    return ns // 1_000_000_000, ns // 1_000_000 % 1000


def format_image(total_blocks, boot_blocks=0, inode_count=256, created=None, capacity=None):
    """A new volume of `capacity` blocks (default total_blocks), §9 format."""
    capacity = total_blocks if capacity is None else capacity
    if not 0 < total_blocks <= min(capacity, MAX_U32):
        raise Fs2Error("FORMAT", "total_blocks must be 1..device capacity")
    pos = layout(total_blocks, boot_blocks, inode_count)
    if pos["data_start"] > total_blocks:
        raise Fs2Error("FORMAT", f"layout needs {pos['data_start']} blocks, volume has {total_blocks}")
    sec, ms = created or now_time()
    img = bytearray(capacity * BLOCK)
    img[BOOT_CODE_SIZE:BLOCK] = BOOT_SIG
    sb = dict(magic=MAGIC, version=VERSION, flags=0, total_blocks=total_blocks, boot_blocks=boot_blocks,
              boot_size=0, inode_count=inode_count, root_inode=ROOT_INODE,
              casefold_version=CASEFOLD_VERSION, created_sec=sec, created_ms=ms, **pos)
    struct.pack_into(SUPER_FMT, img, BLOCK, *(sb[name] for name in SUPER_FIELDS))
    volume = Volume(img, check=False)
    for block in range(pos["data_start"]):
        volume.set_bit(block, True)
    volume.write_inode(ROOT_INODE, volume.new_inode(DIR, MODE_DIR, 0, ROOT_INODE, (sec, ms)))
    return img


class Volume:
    """A mounted FS2 image (bytearray). Mutations change `img` in place; after
    an exception the bytearray may be half-updated and must be discarded."""

    def __init__(self, img, check=True, allow_dirty=False):
        self.img = img
        if check:
            problems = check_image(img, allow_dirty=allow_dirty)
            if problems:
                code, message = problems[0]
                raise Fs2Error(code, message)
        self.sb = dict(zip(SUPER_FIELDS, struct.unpack_from(SUPER_FMT, img, BLOCK)))

    # -- blocks and bitmap (§4) --
    def bit(self, block):
        byte = self.img[self.sb["bitmap_start"] * BLOCK + (block >> 3)]
        return bool(byte & (0x80 >> (block & 7)))

    def set_bit(self, block, used):
        offset = self.sb["bitmap_start"] * BLOCK + (block >> 3)
        if used:
            self.img[offset] |= 0x80 >> (block & 7)
        else:
            self.img[offset] &= ~(0x80 >> (block & 7)) & 0xFF

    def free_blocks(self):
        return [b for b in range(self.sb["data_start"], self.sb["total_blocks"]) if not self.bit(b)]

    def allocate(self, count):
        """First-fit, not necessarily contiguous. Marks the blocks used and zeroes them."""
        found = []
        for block in range(self.sb["data_start"], self.sb["total_blocks"]):
            if len(found) == count:
                break
            if not self.bit(block):
                found.append(block)
        if len(found) < count:
            raise Fs2Error("NO_SPACE", f"need {count} free blocks")
        for block in found:
            self.set_bit(block, True)
            self.img[block * BLOCK:(block + 1) * BLOCK] = bytes(BLOCK)
        return found

    def release(self, blocks):
        for block in blocks:
            self.img[block * BLOCK:(block + 1) * BLOCK] = bytes(BLOCK)
            self.set_bit(block, False)

    def next_block(self, block):
        return struct.unpack_from(">I", self.img, block * BLOCK)[0]

    def link(self, block, following):
        struct.pack_into(">I", self.img, block * BLOCK, following)

    def chain(self, inode):
        blocks, block = [], inode["first_block"]
        while block:
            if len(blocks) >= inode["block_count"]:
                raise Fs2Error("FORMAT", "data chain longer than block_count")
            blocks.append(block)
            block = self.next_block(block)
        if len(blocks) != inode["block_count"]:
            raise Fs2Error("FORMAT", "data chain shorter than block_count")
        return blocks

    # -- inodes (§5) --
    def inode_offset(self, number):
        if not 1 <= number <= self.sb["inode_count"]:
            raise Fs2Error("FORMAT", f"inode {number} out of range")
        return self.sb["inode_start"] * BLOCK + (number - 1) * INODE_SIZE

    def read_inode(self, number):
        values = struct.unpack_from(INODE_FMT, self.img, self.inode_offset(number))
        return dict(zip(INODE_FIELDS, values), number=number)

    def write_inode(self, number, inode):
        offset = self.inode_offset(number)
        self.img[offset:offset + INODE_SIZE] = bytes(INODE_SIZE)
        if inode["type"] != FREE:
            struct.pack_into(INODE_FMT, self.img, offset, *(inode[name] for name in INODE_FIELDS))

    @staticmethod
    def new_inode(kind, mode, uid, parent, stamp):
        sec, ms = stamp
        return dict(type=kind, flags=0, mode=mode, uid=uid, parent=parent, first_block=0, size=0,
                    block_count=0, reserved=0, created_sec=sec, created_ms=ms, modified_sec=sec,
                    modified_ms=ms, changed_sec=sec, changed_ms=ms)

    def free_inode(self):
        for number in range(1, self.sb["inode_count"] + 1):
            if self.img[self.inode_offset(number)] == FREE:
                return number
        raise Fs2Error("LIMIT", "no free inode")

    @staticmethod
    def touch(inode, stamp, modified=True):
        if modified:
            inode["modified_sec"], inode["modified_ms"] = stamp
        inode["changed_sec"], inode["changed_ms"] = stamp

    # -- file data --
    def read(self, inode):
        data = bytearray()
        for block in self.chain(inode):
            data += self.img[block * BLOCK + 4:(block + 1) * BLOCK]
        return bytes(data[:inode["size"]])

    def write(self, inode, offset, data):
        """Writes data at offset, filling any hole with zeros (§9 write)."""
        end = offset + len(data)
        blocks = self.chain(inode)
        needed = -(-max(end, inode["size"]) // PAYLOAD)
        if needed > len(blocks):
            # Prepare the new blocks before linking them in.
            fresh = self.allocate(needed - len(blocks))
            for block, following in zip(fresh, fresh[1:] + [0]):
                self.link(block, following)
            if blocks:
                self.link(blocks[-1], fresh[0])
            else:
                inode["first_block"] = fresh[0]
            blocks += fresh
        position = offset
        while position < end:
            index, inner = divmod(position, PAYLOAD)
            count = min(PAYLOAD - inner, end - position)
            start = blocks[index] * BLOCK + 4 + inner
            self.img[start:start + count] = data[position - offset:position - offset + count]
            position += count
        inode["size"] = max(inode["size"], end)
        inode["block_count"] = len(blocks)

    def truncate(self, inode, size):
        if size >= inode["size"]:
            self.write(inode, inode["size"], bytes(size - inode["size"]))
            return
        blocks = self.chain(inode)
        keep = -(-size // PAYLOAD)
        if keep:
            last = blocks[keep - 1]
            used = size - (keep - 1) * PAYLOAD
            self.img[last * BLOCK + 4 + used:(last + 1) * BLOCK] = bytes(PAYLOAD - used)
            self.link(last, 0)
        else:
            inode["first_block"] = 0
        inode["size"], inode["block_count"] = size, keep
        return blocks[keep:]  # released by the caller after the inode is written

    # -- directories (§6) --
    def entries(self, directory):
        """(slot, inode number, name) for every used entry."""
        data = self.read(directory)
        result = []
        for slot in range(len(data) // ENTRY_SIZE):
            raw = data[slot * ENTRY_SIZE:(slot + 1) * ENTRY_SIZE]
            number, name_bytes, _ = ENTRY_HEAD.unpack_from(raw)
            if number:
                result.append((slot, number, raw[8:8 + name_bytes].decode("utf-8")))
        return result

    def find(self, directory, name):
        key = fold(name)
        for slot, number, entry_name in self.entries(directory):
            if fold(entry_name) == key:
                return slot, number, entry_name
        return None

    def set_entry(self, directory, slot, number, name):
        entry = bytearray(ENTRY_SIZE)
        if number:
            raw = check_name(name)
            ENTRY_HEAD.pack_into(entry, 0, number, len(raw), len(name))
            entry[8:8 + len(raw)] = raw
        self.write(directory, slot * ENTRY_SIZE, bytes(entry))

    def free_slot(self, directory):
        data = self.read(directory)
        for slot in range(len(data) // ENTRY_SIZE):
            if data[slot * ENTRY_SIZE:(slot + 1) * ENTRY_SIZE] == bytes(ENTRY_SIZE):
                return slot
        return len(data) // ENTRY_SIZE

    # -- paths (§11.1, without mounts) --
    def resolve(self, path, want_dir=False):
        """Inode for an absolute path; `.`/`..` are resolved component by component."""
        if not path.startswith("/"):
            raise Fs2Error("BAD_PATH", f"path must be absolute: {path!r}")
        inode = self.read_inode(ROOT_INODE)
        for part in path.split("/"):
            if part in ("", "."):
                continue
            if inode["type"] != DIR:
                raise Fs2Error("NOT_DIR", f"{path}: not a directory")
            if part == "..":
                inode = self.read_inode(inode["parent"])
                continue
            found = self.find(inode, part)
            if not found:
                raise Fs2Error("NOT_FOUND", path)
            inode = self.read_inode(found[1])
        if (want_dir or path.endswith("/")) and inode["type"] != DIR:
            raise Fs2Error("NOT_DIR", f"{path}: not a directory")
        return inode

    def split(self, path):
        """(parent inode, final name) for a path that names a new or existing entry."""
        stripped = path.rstrip("/")
        parent_path, _, name = stripped.rpartition("/")
        if not stripped.startswith("/") or not name:
            raise Fs2Error("BAD_PATH", f"invalid path {path!r}")
        check_name(name)
        return self.resolve(parent_path or "/", want_dir=True), name

    def create(self, path, kind, mode=None, uid=0, stamp=None):
        stamp = stamp or now_time()
        parent, name = self.split(path)
        if self.find(parent, name):
            raise Fs2Error("EXISTS", path)
        number = self.free_inode()
        inode = self.new_inode(kind, MODE_DIR if kind == DIR and mode is None else
                               MODE_FILE if mode is None else mode, uid, parent["number"], stamp)
        self.write_inode(number, inode)
        self.set_entry(parent, self.free_slot(parent), number, name)
        self.touch(parent, stamp)
        self.write_inode(parent["number"], parent)
        return self.read_inode(number)

    def mkdir(self, path, parents=False, mode=None, stamp=None):
        if parents:
            current = ""
            for part in [p for p in path.split("/") if p]:
                current += "/" + part
                try:
                    found = self.resolve(current)
                except Fs2Error as error:
                    if error.code != "NOT_FOUND":
                        raise
                    found = self.create(current, DIR, mode, stamp=stamp)
                if found["type"] != DIR:
                    raise Fs2Error("NOT_DIR", current)
            return self.resolve(path, want_dir=True)
        return self.create(path, DIR, mode, stamp=stamp)

    def put(self, path, data, mode=None, stamp=None):
        """Creates or replaces a regular file."""
        stamp = stamp or now_time()
        parent, name = self.split(path)
        found = self.find(parent, name)
        if found:
            inode = self.read_inode(found[1])
            if inode["type"] != FILE:
                raise Fs2Error("IS_DIR", path)
        else:
            inode = self.create(path, FILE, mode, stamp=stamp)
        self.write(inode, 0, data)
        released = self.truncate(inode, len(data)) or []
        if mode is not None:
            inode["mode"] = mode
        self.touch(inode, stamp)
        self.write_inode(inode["number"], inode)
        self.release(released)
        return inode

    def remove(self, path, stamp=None):
        """unlink or rmdir (empty only); the root cannot be removed."""
        stamp = stamp or now_time()
        parent, name = self.split(path)
        found = self.find(parent, name)
        if not found:
            raise Fs2Error("NOT_FOUND", path)
        inode = self.read_inode(found[1])
        if inode["type"] == DIR and self.entries(inode):
            raise Fs2Error("NOT_EMPTY", path)
        blocks = self.chain(inode)
        self.set_entry(parent, found[0], 0, "")
        self.touch(parent, stamp)
        self.write_inode(parent["number"], parent)
        self.write_inode(inode["number"], {"type": FREE})
        self.release(blocks)

    def rename(self, source, target, stamp=None):
        stamp = stamp or now_time()
        old_parent, old_name = self.split(source)
        found = self.find(old_parent, old_name)
        if not found:
            raise Fs2Error("NOT_FOUND", source)
        inode = self.read_inode(found[1])
        new_parent, new_name = self.split(target)
        clash = self.find(new_parent, new_name)
        if clash and clash[1] != inode["number"]:
            raise Fs2Error("EXISTS", target)
        if clash and clash[2] == new_name:
            return  # the same stored name: success without a change
        if inode["type"] == DIR:
            ancestor = new_parent
            while True:
                if ancestor["number"] == inode["number"]:
                    raise Fs2Error("BAD_PATH", "cannot move a directory into itself")
                if ancestor["number"] == ROOT_INODE:
                    break
                ancestor = self.read_inode(ancestor["parent"])
        if new_parent["number"] == old_parent["number"]:
            self.set_entry(old_parent, found[0], inode["number"], new_name)
            self.touch(old_parent, stamp)
            self.write_inode(old_parent["number"], old_parent)
        else:
            self.set_entry(new_parent, self.free_slot(new_parent), inode["number"], new_name)
            self.touch(new_parent, stamp)
            self.write_inode(new_parent["number"], new_parent)
            old_parent = self.read_inode(old_parent["number"])
            self.set_entry(old_parent, found[0], 0, "")
            self.touch(old_parent, stamp)
            self.write_inode(old_parent["number"], old_parent)
            inode = self.read_inode(inode["number"])
            inode["parent"] = new_parent["number"]
        self.touch(inode, stamp, modified=False)
        self.write_inode(inode["number"], inode)

    def set_attributes(self, path, mode=None, uid=None, stamp=None):
        inode = self.resolve(path)
        if mode is not None:
            if mode & ~MODE_MASK:
                raise Fs2Error("BAD_ARG", "mode uses only bits 0-5 (§8)")
            inode["mode"] = mode
        if uid is not None:
            if not 0 <= uid < INVALID_UID:
                raise Fs2Error("BAD_ARG", "uid must be 0..0xFFFFFFFE")
            inode["uid"] = uid
        self.touch(inode, stamp or now_time(), modified=False)
        self.write_inode(inode["number"], inode)

    def install_boot(self, image, stage1=None):
        """Writes the reserved boot area (§3) and optionally the block 0 boot code."""
        area = self.sb["boot_blocks"] * BLOCK
        if len(image) > area:
            raise Fs2Error("NO_SPACE", f"boot image is {len(image)} bytes, reserved area {area}")
        start = BOOT_START * BLOCK
        self.img[start:start + area] = bytes(image) + bytes(area - len(image))
        self.sb["boot_size"] = len(image)
        struct.pack_into(">I", self.img, BLOCK + 0x14, len(image))
        if stage1 is not None:
            if len(stage1) == BLOCK:
                if stage1[BOOT_CODE_SIZE:] != BOOT_SIG:
                    raise Fs2Error("FORMAT", "512-byte boot sector must end with 55 AA")
                stage1 = stage1[:BOOT_CODE_SIZE]
            if len(stage1) > BOOT_CODE_SIZE:
                raise Fs2Error("FORMAT", f"boot code is {len(stage1)} bytes, at most {BOOT_CODE_SIZE}")
            self.img[:BOOT_CODE_SIZE] = bytes(stage1) + bytes(BOOT_CODE_SIZE - len(stage1))


def check_image(img, allow_dirty=False):
    """Mount validation (§10). Returns a list of (code, message); empty = valid.

    allow_dirty is the fsck inspection mode: DIRTY=1 is accepted, everything
    else is still checked without following unsafe pointers.
    """
    problems = []

    def bad(message, code="FORMAT"):
        problems.append((code, message))

    if len(img) < 2 * BLOCK:
        return [("FORMAT", "image shorter than 2 blocks")]
    if img[BOOT_CODE_SIZE:BLOCK] != BOOT_SIG:
        bad("missing boot signature 55 AA")
    sb = dict(zip(SUPER_FIELDS, struct.unpack_from(SUPER_FMT, img, BLOCK)))
    if sb["magic"] != MAGIC:
        return problems + [("FORMAT", "bad magic")]
    if sb["version"] != VERSION:
        return problems + [("UNSUPPORTED", f"version {sb['version']}")]
    if sb["flags"] & ~FLAG_DIRTY:
        return problems + [("UNSUPPORTED", f"unknown flags 0x{sb['flags']:04X}")]
    if sb["casefold_version"] != CASEFOLD_VERSION:
        return problems + [("UNSUPPORTED", f"casefold_version 0x{sb['casefold_version']:08X}")]
    if sb["flags"] & FLAG_DIRTY and not allow_dirty:
        bad("DIRTY is set: run fsck")
    capacity = len(img) // BLOCK
    total = sb["total_blocks"]
    if not 1 <= sb["inode_count"] <= 0xFFFFFFFE:
        return problems + [("FORMAT", "inode_count out of range")]
    expected = layout(total, sb["boot_blocks"], sb["inode_count"])
    for name, value in expected.items():
        if sb[name] != value:
            bad(f"{name} is {sb[name]}, expected {value}")
    if sb["root_inode"] != ROOT_INODE:
        bad("root_inode must be 1")
    if not expected["data_start"] <= total <= capacity:
        bad(f"need data_start <= total_blocks <= capacity ({expected['data_start']}, {total}, {capacity})")
    if sb["boot_size"] > sb["boot_blocks"] * BLOCK:
        bad("boot_size larger than the reserved boot area")
    if not 0 <= sb["created_ms"] <= 999:
        bad("superblock created_ms out of range")
    if problems:
        return problems  # positions are not trustworthy past this point
    if any(img[BLOCK + SUPER_USED:2 * BLOCK]):
        bad("superblock reserved bytes are not zero")
    boot_end = (BOOT_START + sb["boot_blocks"]) * BLOCK
    if any(img[BOOT_START * BLOCK + sb["boot_size"]:boot_end]):
        bad("boot area padding is not zero")

    volume = Volume(img, check=False)
    for block in range(sb["data_start"]):
        if not volume.bit(block):
            bad(f"metadata block {block} is marked free")
    bitmap_end = (sb["bitmap_start"] + sb["bitmap_blocks"]) * BLOCK * 8
    for index in range(total, bitmap_end - sb["bitmap_start"] * BLOCK * 8):
        if volume.bit(index):
            bad("bitmap padding bits are not zero")
            break
    tail = sb["inode_start"] * BLOCK + sb["inode_count"] * INODE_SIZE
    if any(img[tail:sb["data_start"] * BLOCK]):
        bad("inode table padding is not zero")

    inodes, owner = {}, {}
    for number in range(1, sb["inode_count"] + 1):
        offset = volume.inode_offset(number)
        raw = img[offset:offset + INODE_SIZE]
        inode = volume.read_inode(number)
        if inode["type"] == FREE:
            if any(raw):
                bad(f"free inode {number} is not all zero")
            continue
        label = f"inode {number}"
        if inode["type"] not in (FILE, DIR):
            bad(f"{label}: unknown type {inode['type']}")
            continue
        if inode["flags"] or inode["reserved"] or any(raw[0x2A:0x2C]) or any(raw[0x36:0x38]) or any(raw[INODE_USED:]):
            bad(f"{label}: flags or reserved bytes are not zero")
        if inode["mode"] & ~MODE_MASK:
            bad(f"{label}: undefined mode bits")
        if inode["uid"] == INVALID_UID:
            bad(f"{label}: invalid uid")
        if any(not 0 <= inode[f"{t}_ms"] <= 999 for t in ("created", "modified", "changed")):
            bad(f"{label}: millisecond field out of range")
        if inode["block_count"] != -(-inode["size"] // PAYLOAD) or (inode["size"] == 0) != (inode["first_block"] == 0):
            bad(f"{label}: size, block_count and first_block disagree")
            continue
        blocks, block = [], inode["first_block"]
        while block and len(blocks) <= inode["block_count"]:
            if not sb["data_start"] <= block < total:
                bad(f"{label}: chain leaves the data area")
                break
            if block in owner or block in blocks:
                bad(f"{label}: block {block} is shared or loops")
                break
            if not volume.bit(block):
                bad(f"{label}: block {block} is marked free")
            blocks.append(block)
            block = volume.next_block(block)
        else:
            if len(blocks) != inode["block_count"]:
                bad(f"{label}: chain length differs from block_count")
            elif blocks:
                used = inode["size"] - (len(blocks) - 1) * PAYLOAD
                if any(img[blocks[-1] * BLOCK + 4 + used:(blocks[-1] + 1) * BLOCK]):
                    bad(f"{label}: last block padding is not zero")
        for b in blocks:
            owner[b] = number
        inodes[number] = inode
    for block in range(sb["data_start"], total):
        if volume.bit(block) and block not in owner:
            bad(f"block {block} is used but not referenced")
    if problems:
        return problems

    root = inodes.get(ROOT_INODE)
    if not root or root["type"] != DIR or root["parent"] != ROOT_INODE:
        return problems + [("FORMAT", "root is not a directory whose parent is itself")]
    seen, queue = {ROOT_INODE}, [ROOT_INODE]
    while queue:
        directory = inodes[queue.pop()]
        data = volume.read(directory)
        label = f"directory inode {directory['number']}"
        if len(data) % ENTRY_SIZE:
            bad(f"{label}: size is not a multiple of {ENTRY_SIZE}")
            continue
        keys = set()
        for slot in range(len(data) // ENTRY_SIZE):
            raw = data[slot * ENTRY_SIZE:(slot + 1) * ENTRY_SIZE]
            number, name_bytes, name_chars = ENTRY_HEAD.unpack_from(raw)
            if not number:
                if any(raw):
                    bad(f"{label}: free entry {slot} is not all zero")
                continue
            if not 1 <= name_bytes <= NAME_BYTES or any(raw[8 + name_bytes:]):
                bad(f"{label}: entry {slot} has a bad name length or padding")
                continue
            try:
                name = bytes(raw[8:8 + name_bytes]).decode("utf-8")
                check_name(name)
            except (UnicodeDecodeError, Fs2Error):
                bad(f"{label}: entry {slot} has an invalid name")
                continue
            if len(name) != name_chars:
                bad(f"{label}: entry {slot} name_chars mismatch")
            key = fold(name)
            if key in keys:
                bad(f"{label}: duplicate name {name!r}")
            keys.add(key)
            child = inodes.get(number)
            if child is None or number == ROOT_INODE:
                bad(f"{label}: entry {name!r} points to a free inode or the root")
                continue
            if number in seen:
                bad(f"inode {number} has more than one parent entry")
                continue
            if child["parent"] != directory["number"]:
                bad(f"inode {number}: parent_inode does not match its directory")
            seen.add(number)
            if child["type"] == DIR:
                queue.append(number)
    for number in inodes:
        if number not in seen:
            bad(f"inode {number} is not reachable from the root")
    return problems


def _load(path):
    return bytearray(Path(path).read_bytes())


def _save(path, img):
    path = Path(path)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_bytes(img)
    os.replace(temporary, path)


def _mode(text):
    return int(text, 0)


def _put_tree(volume, source, target, args):
    for host in sorted(source.iterdir()):
        path = target.rstrip("/") + "/" + host.name
        if host.is_dir():
            volume.mkdir(path, parents=True)
            _put_tree(volume, host, path, args)
        else:
            volume.put(path, host.read_bytes(), mode=args.mode)


def _dispatch(args):
    if args.command == "format":
        path = Path(args.image)
        capacity = args.blocks if args.blocks is not None else None
        if capacity is None:
            if not path.exists():
                raise Fs2Error("BAD_ARG", "--blocks is required for a new image")
            capacity = path.stat().st_size // BLOCK
        _save(path, format_image(capacity, args.boot_blocks, args.inodes))
        return 0
    img = _load(args.image)
    if args.command == "check":
        problems = check_image(img, allow_dirty=args.allow_dirty)
        for code, message in problems:
            print(f"{code}: {message}")
        if not problems:
            print("OK")
        return 1 if problems else 0
    volume = Volume(img)
    sb = volume.sb
    if args.command == "info":
        for name in SUPER_FIELDS[1:]:
            value = sb[name]
            print(f"{name:17} {value:#x}" if name == "casefold_version" else f"{name:17} {value}")
        print(f"free_blocks       {len(volume.free_blocks())}")
        return 0
    if args.command == "ls":
        inode = volume.resolve(args.path)
        items = volume.entries(inode) if inode["type"] == DIR else [(0, inode["number"], args.path)]
        for _, number, name in sorted(items, key=lambda item: fold(item[2])):
            child = volume.read_inode(number)
            suffix = "/" if child["type"] == DIR else ""
            print(f"{child['mode']:#06x} {child['uid']:>5} {child['size']:>10} {name}{suffix}")
        return 0
    if args.command in ("get", "cat"):
        inode = volume.resolve(args.path)
        if inode["type"] != FILE:
            raise Fs2Error("IS_DIR", args.path)
        data = volume.read(inode)
        if args.command == "cat":
            sys.stdout.buffer.write(data)
        else:
            Path(args.output).write_bytes(data)
        return 0
    if args.command == "mkdir":
        volume.mkdir(args.path, parents=args.parents, mode=args.mode)
    elif args.command == "put":
        source = Path(args.source)
        if source.is_dir():
            volume.mkdir(args.path, parents=True)
            _put_tree(volume, source, args.path, args)
        else:
            path = args.path.rstrip("/") + "/" + source.name if args.path.endswith("/") else args.path
            volume.put(path, source.read_bytes(), mode=args.mode)
    elif args.command == "rm":
        volume.remove(args.path)
    elif args.command == "mv":
        volume.rename(args.source, args.target)
    elif args.command == "chmod":
        volume.set_attributes(args.path, mode=args.mode)
    elif args.command == "chown":
        volume.set_attributes(args.path, uid=args.uid)
    elif args.command == "install-boot":
        stage1 = Path(args.stage1).read_bytes() if args.stage1 else None
        volume.install_boot(Path(args.boot_image).read_bytes(), stage1)
    problems = check_image(img)
    if problems:  # an operation must never leave an image that does not mount
        raise Fs2Error(*problems[0])
    _save(args.image, img)
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("format", help="create or re-format an image (§9 format)")
    p.add_argument("image")
    p.add_argument("--blocks", type=int, help="volume size in 512-byte blocks (default: the image size)")
    p.add_argument("--boot-blocks", type=int, default=0, help="reserved boot area blocks (default 0)")
    p.add_argument("--inodes", type=int, default=256, help="inode slots including the root (default 256)")
    sub.add_parser("info", help="show the superblock").add_argument("image")
    p = sub.add_parser("check", help="mount validation (§10); fsck inspection with --allow-dirty")
    p.add_argument("image")
    p.add_argument("--allow-dirty", action="store_true")
    p = sub.add_parser("ls", help="list a directory")
    p.add_argument("image")
    p.add_argument("path", nargs="?", default="/")
    for name in ("get", "cat"):
        p = sub.add_parser(name, help="extract a file" if name == "get" else "write a file to stdout")
        p.add_argument("image")
        p.add_argument("path")
        if name == "get":
            p.add_argument("output")
    p = sub.add_parser("put", help="create or replace a file; a host directory is copied recursively")
    p.add_argument("image")
    p.add_argument("source")
    p.add_argument("path", help="target path (ending in / keeps the host name)")
    p.add_argument("--mode", type=_mode, help=f"FS2 mode bits (default {MODE_FILE:#06x})")
    p = sub.add_parser("mkdir", help="create a directory")
    p.add_argument("image")
    p.add_argument("path")
    p.add_argument("-p", "--parents", action="store_true")
    p.add_argument("--mode", type=_mode, help=f"FS2 mode bits (default {MODE_DIR:#06x})")
    p = sub.add_parser("rm", help="remove a file or an empty directory")
    p.add_argument("image")
    p.add_argument("path")
    p = sub.add_parser("mv", help="rename or move within the volume")
    p.add_argument("image")
    p.add_argument("source")
    p.add_argument("target")
    p = sub.add_parser("chmod", help="set FS2 mode bits (§8, e.g. 0x3D)")
    p.add_argument("image")
    p.add_argument("mode", type=_mode)
    p.add_argument("path")
    p = sub.add_parser("chown", help="set the owner UID")
    p.add_argument("image")
    p.add_argument("uid", type=_mode)
    p.add_argument("path")
    p = sub.add_parser("install-boot", help="write the reserved boot area (§3)")
    p.add_argument("image")
    p.add_argument("boot_image")
    p.add_argument("--stage1", help="also write this boot code (510 bytes, or 512 ending in 55 AA) to block 0")
    args = parser.parse_args(argv)
    try:
        return _dispatch(args)
    except (Fs2Error, OSError) as error:
        print(f"fs2: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
