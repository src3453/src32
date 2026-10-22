"""WAX/WAL v3 image builder (doc/spec_WAX_v3.md).

There is no v3 compiler or linker yet. This module lays out a v3 file from
blocks and symbol tables that the caller already has (for example from
tools/asm), and is what the Foundation loader tests use. It writes what it is
given: rule checks belong to the loader, so tests can build invalid images
and patch fields through `Image.layout`.
"""
from dataclasses import dataclass
import struct

MAGIC = b"WAX\x00"
VERSION = 3
TYPE_WAX, TYPE_WAL = 1, 2
ARCH_SRC32 = 1
FLAG_DEBUG, FLAG_PROTECTED = 1, 2
HEADER_SIZE = 160
CALL_ABI = 1
PERM_R, PERM_RW, PERM_RX = 1, 3, 5
FUNCTION, OBJECT = 1, 2


@dataclass
class Block:
    address: int  # WAX: absolute; WAL: offset from the library base
    data: bytes = b""
    memory_size: int | None = None  # default: len(data)
    perm: int = PERM_RX
    alignment: int = 4


@dataclass
class Dependency:
    library_id: str
    abi_major: int = 1
    min_minor: int = 0


@dataclass
class Export:
    name: str
    kind: int
    offset: int  # image-relative
    size: int
    signature: str | None = None


@dataclass
class Import:
    dependency: int
    name: str
    kind: int
    slot: int  # image-relative offset of the 8-byte descriptor
    size: int = 0
    signature: str | None = None


@dataclass
class Image:
    data: bytearray
    layout: dict  # file offsets: 'blocks', 'deps', 'exports', 'imports', 'strings', 'payloads', 'debug'

    def put32(self, offset, value):
        struct.pack_into(">I", self.data, offset, value & 0xFFFFFFFF)

    def get32(self, offset):
        return struct.unpack_from(">I", self.data, offset)[0]


def _align4(n):
    return (n + 3) & ~3


class _Strings:
    def __init__(self):
        self.table = bytearray(b"\0")
        self.offsets = {}

    def add(self, text):
        if text is None:
            return 0
        if text not in self.offsets:
            self.offsets[text] = len(self.table)
            self.table += text.encode("ascii") + b"\0"
        return self.offsets[text]


def build(kind="wax", *, image_id, blocks, abi_major=1, abi_minor=0, entry_offset=0,
          stack_top=0, stack_size=0, deps=(), exports=(), imports=(), os_id="foundation",
          os_major=1, os_min_minor=0, debug=b"", call_abi=CALL_ABI, protected=True):
    """Returns an Image. Tables are written in the order given."""
    file_type = {"wax": TYPE_WAX, "wal": TYPE_WAL}[kind]
    strings = _Strings()
    image_id_offset = strings.add(image_id)
    os_id_offset = strings.add(os_id)
    dep_rows = [(strings.add(d.library_id), d.abi_major, d.min_minor, 0) for d in deps]
    export_rows = [(strings.add(e.name), e.kind, e.offset, e.size, strings.add(e.signature), 0)
                   for e in exports]
    import_rows = [(i.dependency, strings.add(i.name), i.kind, i.slot, i.size, strings.add(i.signature))
                   for i in imports]

    layout = {}
    at = HEADER_SIZE
    for name, count, size in (("blocks", len(blocks), 24), ("deps", len(dep_rows), 16),
                              ("exports", len(export_rows), 24), ("imports", len(import_rows), 24)):
        layout[name] = at if count else 0
        at += count * size
    layout["strings"] = at
    at = _align4(at + len(strings.table))
    layout["payloads"] = []
    for block in blocks:
        layout["payloads"].append(at if block.data else 0)
        at = _align4(at + len(block.data))
    layout["debug"] = at if debug else 0
    at += len(debug)

    out = bytearray(at)
    base = blocks[0].address if file_type == TYPE_WAX and blocks else 0
    flags = (FLAG_PROTECTED if protected else 0) | (FLAG_DEBUG if debug else 0)
    struct.pack_into(">4sHHHHIIIIIII", out, 0, MAGIC, VERSION, file_type, ARCH_SRC32, flags,
                     HEADER_SIZE, at, call_abi, base, entry_offset, stack_top, stack_size)
    struct.pack_into(">" + "I" * 22, out, 40,
                     layout["blocks"], len(blocks), 24,
                     layout["deps"], len(dep_rows), 16,
                     layout["exports"], len(export_rows), 24,
                     layout["imports"], len(import_rows), 24,
                     layout["strings"], len(strings.table),
                     image_id_offset, abi_major, abi_minor, os_id_offset, os_major, os_min_minor,
                     layout["debug"], len(debug))
    for i, block in enumerate(blocks):
        memory_size = len(block.data) if block.memory_size is None else block.memory_size
        struct.pack_into(">6I", out, layout["blocks"] + 24 * i, block.address,
                         layout["payloads"][i], len(block.data), memory_size, block.perm, block.alignment)
        out[layout["payloads"][i]:layout["payloads"][i] + len(block.data)] = block.data
    for table, rows, size in (("deps", dep_rows, 16), ("exports", export_rows, 24),
                              ("imports", import_rows, 24)):
        for i, row in enumerate(rows):
            struct.pack_into(">%dI" % (size // 4), out, layout[table] + size * i, *row)
    out[layout["strings"]:layout["strings"] + len(strings.table)] = strings.table
    out[layout["debug"]:layout["debug"] + len(debug)] = debug
    return Image(out, layout)


@dataclass
class Library:
    """What a linker needs from a WAL: its identity and exports."""
    image_id: str
    abi_major: int
    abi_minor: int
    call_abi: int
    os_id: str
    os_major: int
    exports: list


def read_library(data):
    """Reads a WAL's identity and export table. Checks only what reading
    needs; the loader still validates the whole file (spec §2-§4)."""
    data = bytes(data)
    if len(data) < HEADER_SIZE or data[:4] != MAGIC:
        raise ValueError("not a WAX/WAL file")
    version, file_type = struct.unpack_from(">HH", data, 4)
    if version != VERSION or file_type != TYPE_WAL:
        raise ValueError("not a WAX v3 library (WAL)")
    words = struct.unpack_from(">40I", data, 0)
    call_abi = words[5]
    export_offset, export_count, export_size = words[16], words[17], words[18]
    string_offset, string_size = words[22], words[23]
    if export_size != 24 or string_offset + string_size > len(data) \
            or export_offset + export_count * 24 > len(data):
        raise ValueError("malformed WAL tables")
    strings = data[string_offset:string_offset + string_size]

    def text(offset):
        if offset == 0:
            return None
        end = strings.find(b"\0", offset)
        if offset >= len(strings) or end < 0:
            raise ValueError("bad string offset in WAL")
        return strings[offset:end].decode("ascii")

    exports = []
    for i in range(export_count):
        name, kind, offset, size, signature, _ = struct.unpack_from(">6I", data, export_offset + 24 * i)
        exports.append(Export(text(name), kind, offset, size, text(signature)))
    return Library(text(words[24]), words[25], words[26], call_abi, text(words[27]), words[28], exports)
