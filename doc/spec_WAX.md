# WAX (Where's All the eXecutables?) container spec v2

WAX is a sparse SRC32 executable container. `.wax` is an executable and
`.wal` is a PIC library without relocation. The format uses a block table so
unmapped address gaps consume no file space. v2 is incompatible with v1;
v1 files must be repacked. No v1 compatibility parser is provided.

## Layout

The fixed 64-byte header is followed by a block table, aligned payload blocks,
and an optional debug block. All multi-byte integers are big-endian. Payload
bytes are opaque.

```
Header offset  size  name
0             4     magic = 57 41 58 00 ("WAX\0")
4             2     version = 2
6             2     type: 1 = .wax executable, 2 = .wal library
8             2     arch = 1 (SRC32)
10            2     flags: bit0 = DEBUG_PRESENT, others 0
12            4     header_size = 64
16            4     base_addr: lowest .wax block address; 0 for .wal
20            4     entry_offset: entry = base_addr + entry_offset; 0 for .wal
24            4     block_table_offset
28            4     block_count
32            4     block_entry_size = 12
36            4     debug_offset (0 when absent)
40            4     debug_size (0 when absent)
44            4     file_size (exact byte length)
48            16    reserved (zero)

Block table entry (12 bytes each):
0             4     load_address: u32 address in .wax; PIC offset in .wal
4             4     file_offset: payload position in the container
8             4     size: payload byte length
```

## Rules

- `block_table_offset` is 4-byte aligned and at least 64. `block_count` is
  nonzero; `block_entry_size` is exactly 12. The full table must fit in the file.
- Every block has nonzero size. Table entries are sorted by `load_address`,
  and loaded address ranges must not overlap or wrap past `0x1_0000_0000`.
- Each `file_offset` is 4-byte aligned and begins after the table. Payload
  ranges must fit within `file_size` and must not overlap each other or debug.
  `file_size` must equal the actual file length.
- `.wax` `base_addr` equals the first block's `load_address`. The entry
  `base_addr + entry_offset` must not wrap u32 and must lie inside one block;
  it cannot point into a hole.
- `.wal` `base_addr` and `entry_offset` MUST be 0. Block addresses are PIC
  offsets. Libraries are parsed but cannot be executed.
- `DEBUG_PRESENT` is set iff `debug_size > 0`. Debug is NOLOAD and opaque.
- Unknown version, architecture, type, or flag bits are rejected. v2 has no
  checksum.

## Loading

1. Inputs without the WAX magic remain legacy raw `.bin`: load at address 0,
   entry PC 0.
2. A WAX loader writes each payload to its table `load_address`. Address gaps
   are not present in the file and are not written by the loader.
3. `.wax` starts at `base_addr + entry_offset`. Debug is skipped.
4. CPT32PKG entrypoints may still name `.wax` or raw `.bin`; the manifest is
   unchanged.

## Test vectors

- One block: NOP bytes at address 0, `base_addr=0`, `entry_offset=0`.
- Sparse image: NOP at `0x100`, HALT at `0x10000`; two 4-byte blocks occupy
  96 bytes total (64-byte header + 24-byte table + 8-byte payload), instead of
  storing the intervening zero-filled address range.
- Reject: empty table, unsorted/overlapping load ranges, an entry in a gap,
  payload/table overlap, out-of-file payload, address wrap, bad version.
- `.wal`: type=2, base=entry=0, one or more PIC blocks; executable loading is
  refused.

## Tooling

- `tools/wax/wax.py pack --base 0x100 --block 0x100=code.bin
  --block 0x10000=stub.bin -o app.wax` creates a multi-block image.
- `tools/asm/asm.py input.asm -o output.wax [--entry LABEL]` writes one block
  per separated `.ORG` region. Gaps are omitted; `.ORG` addresses are retained
  as load addresses. `.wax` selects WAX automatically; other output suffixes
  retain raw-binary output.
- `src/wax.rs` parses and loads blocks. Main, testbench, monitor, and BIOS
  consumers use those block load addresses.
- `tools/imhex/wax.hexpat` visualizes the v2 header, block table, payload bytes
  at their file offsets, and the optional debug block.
