# WAX (Where's All the eXecutables?) container spec v1

WAX is a small ELF-like container for SRC32. Extensions:
`.wax` = executable, `.wal` = library (same format, type differs).
WAX stands for "Where's All the eXecutables?".
WAL stands for "Where's All the Libraries?" and is a PIC lump with no relocation in v1.
This spec covers v1 container only. No link/relocation yet.

## Layout

64-byte header, then text bytes, rodata bytes, debug bytes.
Field order and offsets are fixed. All multi-byte header
fields are little-endian. Payload bytes are opaque.

```
offset  size  name
0       4     magic = 57 41 58 00 ("WAX\0")
4       2     version = 1
6       2     type: 1 = .wax executable, 2 = .wal library
8       2     arch = 1 (SRC32)
10      2     flags: bit0 = DEBUG_PRESENT, others 0
12      4     header_size = 64
16      4     base_addr (executable load base; ignored for .wal)
20      4     entry_offset (entry = base + entry_offset; ignored for .wal)
24      4     text_offset
28      4     text_size (must be > 0)
32      4     rodata_offset
36      4     rodata_size (0 = absent)
40      4     debug_offset
44      4     debug_size (0 = absent)
48      16    reserved (zero)

## Rules

- Offsets are 4-byte aligned. Blobs lie inside the file,
  never overlap, never wrap past EOF.
- text_size > 0. rodata/debug may be empty (size 0 means
  the offset is ignored).
- DEBUG_PRESENT set iff debug_size > 0.
- base_addr + text/rodata extent must not wrap u32.
  Main RAM is 0x00000000-0x00FFFFFF (16 MB).
- .wal: base_addr and entry_offset MUST be 0 (ignored).
  text/rodata are PIC lumps. No relocation in v1.
- .wax entry = base_addr + entry_offset, must land
  inside the loaded text extent.
- Unknown version/arch/type/flag bits: reject.
- No checksum in v1. Corruption check is magic +
  version + range validation.
- Debug block is NOLOAD and opaque in v1. Loader skips it.

## Loading

1. If first 4 bytes != magic, treat input as legacy raw
   .bin: load at address 0, PC = 0.
2. Else parse header, validate, then write text bytes to
   base_addr+i and rodata bytes to base_addr+text_size+i
   (rodata follows text in memory).
3. Set PC = base_addr + entry_offset (.wax only).
4. CPT32PKG entrypoint path may name .wax or .bin;
   loader sniffs magic. Manifest schema is unchanged.

## Test vectors

- Minimal .wax: text = 4 bytes NOP (0x00000000),
  base 0, entry_offset 0, no rodata/debug. Loads at 0, PC=0.
- Offset .wax: text at base 0x100, entry_offset 0.
  Loader writes to 0x100, PC=0x100.
- .wal: type=2, base=entry=0, text present. Parse OK,
  executable load refused (library, not runnable).
- Reject: bad magic version, overlap, misaligned offset,
  empty text, entry outside text, truncated blob.

## Tooling (v1)

- tools/wax/wax.py: pack/unpack/info for .wax/.wal.
- src/wax.rs: parse + load used by main/testbench/monitor.
- asm.py unchanged (still emits raw .bin; wax.py composes).
```
