# WAX (Where's All the eXecutables?) container spec v2

This page describes the implemented v2 format. The separately specified
[WAX/WAL v3](spec_WAX_v3.md) adds mandatory CPU protection, dependencies and
library symbols; the Foundation kernel loads v3, but no compiler generates it
yet.

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
10            2     flags: bit0 = DEBUG_PRESENT, bit1 = PROTECTED, others 0
12            4     header_size = 64
16            4     base_addr: lowest .wax block address; 0 for .wal
20            4     entry_offset: entry = base_addr + entry_offset; 0 for .wal
24            4     block_table_offset
28            4     block_count
32            4     block_entry_size = 12 (16 when PROTECTED)
36            4     debug_offset (0 when absent)
40            4     debug_size (0 when absent)
44            4     file_size (exact byte length)
48            4     user_stack_top (PROTECTED only; otherwise 0)
52            4     user_stack_size (PROTECTED only; otherwise 0)
56            8     reserved (zero)

Block table entry (12 bytes each, 16 when PROTECTED):
0             4     load_address: u32 address in .wax; PIC offset in .wal
4             4     file_offset: payload position in the container
8             4     size: payload byte length
12            4     perm (PROTECTED only): bit0 R, bit1 W, bit2 X
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
- `PROTECTED` rules are in "Protected executables" below.

## Protected executables

`PROTECTED` (flags bit1) marks a `.wax` that runs in Extension P **user mode**
(`doc/spec_CPU_priv.md`). Images without the flag are unchanged and run in
supervisor mode as before. The flag adds no new version: v2 loaders that do
not know it reject the file, so a protected program never runs unprotected by
accident.

Format rules when `PROTECTED` is set:

- Type must be `.wax`. A `.wal` with `PROTECTED` is rejected.
- `block_entry_size` is 16. Each entry's `perm` is a nonzero mask of R (1),
  W (2), X (4); other bits are rejected. `perm` uses the same bits as an
  Extension P region `FLAGS` word.
- `block_count` is at most 7 (region 0 is the stack, regions 1..7 the blocks).
- `user_stack_top` and `user_stack_size` are nonzero multiples of 4.
  The stack is `user_stack_top - user_stack_size .. user_stack_top`. It must
  not underflow address 0, must not overlap a block, and must end at or below
  `0xFFFF0000`.
- No block may reach the CPU-reserved area `0xFFFF0000..`.
- The entry must lie in a block with X.
- Header bytes 56..63 stay zero. Without `PROTECTED`, bytes 48..63 stay zero
  and `block_entry_size` stays 12.

### Hand-off to user mode

The machine always resets in supervisor mode. The **BIOS WAX loader** is the
component that enters user mode. For a protected image it:

1. Validates the file and writes every block to its `load_address`.
2. Programs the protection regions (`RGNn_START/END/FLAGS`):
   - region 0: the user stack, `R|W`;
   - region `1 + i`: block `i` in table order, with the block's `perm`;
   - every other region: all zero (disabled).
3. Sets `USP = user_stack_top`, `EPC = entry`, and `STATUS.PU = 1`. `PIE`
   selects whether interrupts are enabled in user mode.
4. Executes `IRET`.

Payload blocks are copied from the file only; the loader does not clear the
stack or any gap. Devices are not granted. A protected program reaches devices
through `SYSCALL`, and the BIOS/OS that installed the SYSCALL vector defines
those services.

Current status:

- `src/wax.rs` and `tools/wax/wax.py` parse and validate protected images and
  expose the region list (`WaxImage::user_regions`, `user_regions(image)`).
- The BIOS WAX loader is not implemented yet. Host direct loading (main,
  testbench, monitor) starts programs in supervisor mode only, so it refuses
  protected images with an error instead of running them unprotected. The
  BIOS ROM image itself must not be `PROTECTED`.
- `sol/cpt32/wax.sol` (DOS `RUN`) rejects the unknown flag.

## Loading

1. Inputs without the WAX magic remain legacy raw `.bin`: load at address 0,
   entry PC 0.
2. A WAX loader writes each payload to its table `load_address`. Address gaps
   are not present in the file and are not written by the loader.
3. `.wax` starts at `base_addr + entry_offset`. Debug is skipped.
4. CPT32PKG entrypoints may still name `.wax` or raw `.bin`; the manifest is
   unchanged.

## Test vectors

- Protected: code `R|X` at `0x100000`, data `R|W` at `0x110000`, stack
  `0x1FF000..0x200000`. Regions are stack, code, data. Reject a non-X entry
  block, a zero or unknown permission bit, a stack overlapping a block or the
  CPU-reserved area, a misaligned stack, more than 7 blocks, and `.wal`.

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
- `tools/wax/wax.py pack --protected --base 0x100000
  --block 0x100000=code.bin:rx --block 0x110000=data.bin:rw
  --stack-top 0x200000 --stack-size 0x1000 -o app.wax` creates a protected
  image. `info` prints the stack and each block's permission.
- `tools/asm/asm.py input.asm -o output.wax [--entry LABEL]` writes one block
  per `.ORG` address region. A `.ORG` at the current PC does not move the start
  address of bytes already emitted. Gaps are omitted; expanding blocks into a
  zero-filled image at their load addresses matches the raw binary output.
  `.wax` selects WAX automatically; other output suffixes retain raw-binary output.
  `.PROTECTED stack_top, stack_size` makes the `.wax` PROTECTED; `.PERM rwx` starts
  a new block with those permissions, also between adjacent addresses (see
  [CPU §7.1](spec_CPU.md)). `solc` emits them for `!abi_compat "Foundation"`.
- `src/wax.rs` parses and loads blocks. Main, testbench, monitor, and BIOS
  consumers use those block load addresses.
- `sol/cpt32/wax.sol` validates WAX v2 executables in guest RAM and loads
  their sparse blocks before transferring control to the declared entry. DOS
  exposes this as `RUN NAME`; DOS limits files to 64 KiB and rejects blocks
  overlapping `0x00FE0000..0x01000000`. DOS reserves that RAM for staging, an
  8 KiB file-span bitmap, and the transfer trampoline.
- `tools/imhex/wax.hexpat` visualizes the v2 header, block table, payload bytes
  at their file offsets, and the optional debug block.
