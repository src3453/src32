"""FNDBOOT boot image packer (doc/spec_Foundation.md §11).

Layout: the 64-byte header, the module table (48 bytes per module), each
module's grant array (12 bytes per grant: start, end, perm, as in
proc_spawn §9.2), then the kernel raw image and the module images. Every
part starts on a 4-byte boundary. All words are big-endian.
"""
import struct

MAGIC = 0x464E4442  # "FNDB"
VERSION = 1
HEADER = struct.Struct(">9I28x")
ENTRY = struct.Struct(">6I8x16s")
GRANT = struct.Struct(">3I")
KERNEL_LOAD = 0x00010000
KERNEL_MAX = 0x000E0000  # kernel image area, §5.1
MAX_SIZE = 2 * 1024 * 1024
MAX_GRANTS = 8           # FND_MAX_GRANTS


def _align(data):
    return data + bytes(-len(data) % 4)


def pack(kernel, modules, kernel_entry=KERNEL_LOAD, kernel_load=KERNEL_LOAD):
    """kernel: raw image bytes for kernel_load. modules: dicts with name,
    image, priority, caps (default 0) and grants [(start, end, perm)], in
    spawn order; §11 requires init to be the last one."""
    if kernel and not kernel_load <= kernel_entry < kernel_load + len(kernel):
        raise ValueError("kernel_entry is outside the kernel image")
    if len(kernel) > KERNEL_MAX:
        raise ValueError(f"kernel image is {len(kernel)} bytes, at most {KERNEL_MAX:#x}")
    if not modules:
        raise ValueError("FNDBOOT needs at least one module")
    table_off = HEADER.size
    offset = table_off + ENTRY.size * len(modules)
    grant_offsets = []
    for module in modules:
        grants = module.get("grants", ())
        if len(grants) > MAX_GRANTS:
            raise ValueError(f"{module['name']}: more than {MAX_GRANTS} grants")
        grant_offsets.append(offset if grants else 0)
        offset += GRANT.size * len(grants)
    kernel_off = offset
    body = bytearray()
    body += _align(bytes(kernel))
    image_offsets = []
    for module in modules:
        image_offsets.append(kernel_off + len(body))
        body += _align(bytes(module["image"]))
    size = kernel_off + len(body)
    if size > MAX_SIZE:
        raise ValueError(f"FNDBOOT is {size} bytes, at most {MAX_SIZE}")

    out = bytearray(HEADER.pack(MAGIC, VERSION, size, kernel_off, len(kernel), kernel_load,
                                kernel_entry, len(modules), table_off))
    for module, image_off, grant_off in zip(modules, image_offsets, grant_offsets):
        name = module["name"].encode("ascii")
        if not 0 < len(name) < 16:
            raise ValueError(f"module name {module['name']!r} must be 1..15 ASCII bytes")
        out += ENTRY.pack(image_off, len(module["image"]), module["priority"], module.get("caps", 0),
                          grant_off, len(module.get("grants", ())), name)
    for module in modules:
        for grant in module.get("grants", ()):
            out += GRANT.pack(*grant)
    assert len(out) == kernel_off
    return bytes(out + body)


def unpack(data):
    """The header fields and module entries of an FNDBOOT image (for tests and tools)."""
    fields = HEADER.unpack_from(data)
    header = dict(zip(("magic", "version", "size", "kernel_off", "kernel_size", "kernel_load",
                       "kernel_entry", "module_count", "module_table_off"), fields))
    modules = []
    for index in range(header["module_count"]):
        values = ENTRY.unpack_from(data, header["module_table_off"] + index * ENTRY.size)
        module = dict(zip(("offset", "size", "priority", "caps", "grant_off", "grant_count"), values[:6]))
        module["name"] = values[6].rstrip(b"\0").decode("ascii")
        module["grants"] = [GRANT.unpack_from(data, module["grant_off"] + g * GRANT.size)
                            for g in range(module["grant_count"])]
        modules.append(module)
    return header, modules
