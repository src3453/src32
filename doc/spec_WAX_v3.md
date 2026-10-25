# WAX/WAL v3 — protected executables and library symbols

Revision 3.1, 2026-10-25. Status: **specified; loaded by the Foundation kernel**. This document
defines the v3 wire format and the baseline OS loader contract. MUST, MUST NOT
and MAY are normative. [Japanese translation](ja/spec_WAX_v3.md).

The existing [v2 specification](spec_WAX.md), parsers, assembler output, C
toolchain and DOS loader retain their current behavior. The Foundation kernel
loads v3 (`foundation/kernel/wax.c`; its selection and placement policy is in
[Foundation §9.5](spec_Foundation.md)). `tools/llvm-src32` compiles C to
v3 WAX and WAL with call ABI 1 (`--wax3`, `--wal`, `--import-wal`; see its
README). `tools/wax/wax3.py` lays out a v3 file from prepared blocks and
symbol tables; the compiler and the loader tests use it. v3 uses the same magic with `version = 3`;
v2 readers must reject it. Changing the version field of a v2 file is not
conversion.

## 1. Scope and identities

- WAX is a fixed-address **user executable**; it declares its stack, blocks
  and direct library dependencies. Static-linked code is part of its blocks,
  not a runtime dependency.
- WAL is a **position-independent user library**. It has no process entry or
  stack and cannot be launched. It declares its own direct dependencies and
  exports. It never executes in S mode on behalf of a user program.
- Every image uses Extension P protection. Flags bit 1 `PROTECTED` is mandatory
  for both types. Supervisor BIOS/kernel images continue to use v2/raw images;
  this version does not define supervisor modules.
- Library identity is `(image_id, abi_major)`, independent of filename/path.
  `abi_minor >= min_minor` satisfies a dependency; minor revisions MUST retain
  the old ABI and semantics. Breaking changes require a different major.
- The container version, call ABI (`call_abi`), library ABI major/minor and
  OS syscall ABI (`os_id`, `os_major`, `os_min_minor`) are separate identities.
  An OS MUST reject an unsupported one, never infer compatibility from names.

The baseline supports eager binding, function and object exports, private
per-process WAL instances and an acyclic dependency graph. Lazy binding,
weak/global/interposed symbols, TLS, symbol relocations, automatic constructors,
finalizers, runtime load/unload and cross-process sharing are not defined.
Libraries needing setup/teardown expose ordinary explicit functions.

## 2. Representation and header

All integers are unsigned big-endian. Offsets and addresses are in bytes;
ranges are half-open. All arithmetic, including `count * entry_size`, MUST be
checked without u32 wrap before narrowing. File length MUST fit u32.

The header is exactly 160 bytes. Table sizes are mandatory even for an empty
table. An absent table has offset=count=0; other offsets are file-relative
unless explicitly called image-relative. Reserved fields MUST be zero.

| Offset | Size | Field / required value |
|---:|---:|---|
| 0 | 4 | magic `57 41 58 00` (`WAX\0`) |
| 4 | 2 | version = 3 |
| 6 | 2 | type: 1 WAX, 2 WAL |
| 8 | 2 | arch = 1 (SRC32) |
| 10 | 2 | flags: bit 0 DEBUG_PRESENT, bit 1 PROTECTED, bit 2 RELOCATABLE (§3.1); others zero |
| 12 | 4 | header_size = 160 |
| 16 | 4 | file_size = exact file length |
| 20 | 4 | call_abi = 1 (§6) |
| 24 | 4 | base_addr: WAX lowest block address; WAL zero |
| 28 | 4 | entry_offset: WAX image-relative Normal-mode entry; WAL zero |
| 32 | 4 | user_stack_top: WAX absolute exclusive end; WAL zero |
| 36 | 4 | user_stack_size: WAX nonzero; WAL zero |
| 40 | 4 | block_table_offset |
| 44 | 4 | block_count: nonzero |
| 48 | 4 | block_entry_size = 24 |
| 52 | 4 | dependency_table_offset |
| 56 | 4 | dependency_count |
| 60 | 4 | dependency_entry_size = 16 |
| 64 | 4 | export_table_offset |
| 68 | 4 | export_count: WAX zero; WAL nonzero |
| 72 | 4 | export_entry_size = 24 |
| 76 | 4 | import_table_offset |
| 80 | 4 | import_count |
| 84 | 4 | import_entry_size = 24 |
| 88 | 4 | string_table_offset |
| 92 | 4 | string_table_size: nonzero |
| 96 | 4 | image_id: string offset |
| 100 | 4 | abi_major: nonzero; WAX application contract version |
| 104 | 4 | abi_minor |
| 108 | 4 | os_id: syscall ABI string offset |
| 112 | 4 | os_major: nonzero |
| 116 | 4 | os_min_minor |
| 120 | 4 | debug_offset: zero when absent |
| 124 | 4 | debug_size: zero when absent |
| 128 | 4 | reloc_table_offset: file-relative; nonzero iff RELOCATABLE |
| 132 | 4 | reloc_count: nonzero iff RELOCATABLE |
| 136 | 4 | heap_hint: bytes of heap the program asks for, multiple of 8; zero when absent; only with RELOCATABLE |
| 140 | 20 | reserved |

Header, tables (including the relocation table), string table, nonempty block payloads and optional debug are
pairwise disjoint file ranges wholly inside `file_size`. All start offsets
are multiples of 4. They may appear in any order after the header. Padding is
zero. Debug is opaque, NOLOAD; DEBUG_PRESENT is set iff debug_size > 0.
Unknown versions, types, architectures, flags and ABI IDs MUST be rejected.
There is no checksum or authenticity claim.

### 2.1 Strings

The string table starts with a NUL byte. A string offset is relative to that
table, must point to the start of a nonempty NUL-terminated string (preceded
by NUL), and must terminate within the table. Strings are case-sensitive ASCII,
with no normalization. Offset 0 denotes no string only where allowed below.
Unused bytes must form complete NUL-terminated strings or zero padding.

Image, dependency and OS IDs match `[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*`
and are at most 127 bytes, excluding NUL. Symbol names match
`[A-Za-z_][A-Za-z0-9_]*`, at most 127 bytes. IDs are not paths: OS-controlled
lookup maps an ID/major to a file. The image cannot specify a search path.

## 3. Blocks and address domains

Every 24-byte block entry contains six u32 words:

| Offset | Field | Meaning |
|---:|---|---|
| 0 | address | WAX absolute address; WAL offset from allocated base B |
| 4 | file_offset | payload offset; zero iff file_size is zero |
| 8 | file_size | initialized byte count |
| 12 | memory_size | allocated size, nonzero, >= file_size |
| 16 | perm | exactly R=1, RW=3 or RX=5 |
| 20 | alignment | power of two, at least 4; address divisible by it |

Blocks are sorted by address and their memory ranges MUST NOT overlap.
For WAX, `base_addr` equals the first address. For WAL the first address MUST
be zero; the loader selects B aligned to the maximum block alignment.
Its reservation spans `[B, B + max(address + memory_size))`, including holes.
For WAX the reservation spans its lowest through highest block, including
holes. Reservations of different images and the stack MUST NOT overlap.

Define `origin = base_addr` for WAX and `origin = B` for WAL. All
**image-relative offsets** (entry, symbols, import slots) resolve to
`origin + offset`. WAX block address does not receive origin a second time.
For a WAL block the destination is `B + address`.

Copy `file_size` bytes, then zero the tail through `memory_size`. Holes are
unmapped, not implicitly cleared or granted. File metadata is never implicitly
mapped. All allocated ranges MUST be inside actual main RAM and outside the
OS, other processes, loader staging and other reserved memory. Files cannot
request VRAM, device, ROM or CPU-reserved mappings. W+X is prohibited.

WAX entry MUST be 4-byte aligned and its entire first 4-byte instruction must
lie in initialized RX bytes. Stack top and size MUST be nonzero multiples of
8, without underflow, and the stack MUST NOT overlap any image reservation.
The OS zeros the entire stack before use. WAL stack and entry fields are zero.

### 3.1 Relocatable executables

A WAX with flag RELOCATABLE can be loaded at any address, as in a nommu
system's FLAT format. `base_addr`, block addresses, the entry and the stack are
the **link-time** layout; the loader may place the whole image at
`base_addr + delta` for any `delta` that is a multiple of the maximum block
alignment and keeps every range inside RAM, and then applies the relocation
table. Only WAX can be relocatable.

The relocation table is `reloc_count` u32 entries at `reloc_table_offset`,
sorted by strictly increasing address. Each entry is a link-time address in
the high 30 bits (4-byte aligned) and a kind in the low 2 bits:

| Kind | Meaning |
|---:|---|
| 0 | a 32-bit big-endian data word in an R or RW block; add `delta` |
| 1 | an `LDIH` rd, hi followed by `LDIL` rd, lo (same rd) in an RX block; the 32-bit value `hi:lo` is a link-time address; add `delta` and rewrite both immediates |

Other kinds are reserved and MUST be rejected. Every entry MUST lie wholly in
the initialized bytes of one block of the matching permission. The entry
point, image-relative offsets, and the import slots are relative to the
placed origin and need no entry. Relocation happens before the process runs
and before the stack is cleared; it does not apply to WALs (which are
position independent by construction).

The stack of a relocatable WAX lies immediately below the lowest block
(`user_stack_top == base_addr`), so the whole reservation stays one
contiguous range `[base_addr - user_stack_size, highest block end)` that
moves as a unit. The loader sets the initial `R30` to the placed origin
(`base_addr + delta`), which lets position-independent start code find the
image. A loader MAY give the process extra zeroed memory above the highest
block for its heap (see `heap_hint`); that memory is not part of the image.

## 4. Dependencies and symbol tables

### 4.1 Dependency entry (16 bytes)

Four u32 words: `library_id`, `abi_major`, `min_minor`, `reserved=0`.
`library_id` is a string offset; major is nonzero. Array index is the dependency
index used by imports. Duplicate library IDs and self-dependencies are invalid.
Every dependency MUST be used by at least one import. Both WAX and WAL may
import; only WAL exports.

The loader resolves the full graph before any user instruction runs. A process
has at most one instance per library ID. Different requested majors for the
same ID are a conflict; minor requirements combine by maximum. The OS chooses
one installed provider satisfying the combined constraints, with a documented
deterministic selection policy, and records the selected provider/version.
The selected file's own image_id/abi_major/abi_minor MUST satisfy the request;
a matching filename or catalog entry alone is insufficient.
All images must use the same call ABI and OS ID/major; the OS's supported minor
must meet every image's os_min_minor. Cycles and unsatisfied dependencies fail
the entire launch. This is not a global namespace: each import sees only its
explicit dependency's own exports, never a transitive dependency's exports.

### 4.2 Export entry (24 bytes, WAL only)

Six u32 words: `name`, `kind`, `value_offset`, `size`, `signature`, `flags=0`.
Name is a symbol string offset; exports are sorted by ASCII name, with no
duplicates even across kinds. `kind=1` FUNCTION or `kind=2` OBJECT.

- FUNCTION: signature is mandatory (§6), size is nonzero and a multiple of 4.
  value_offset is 4-byte aligned; the full extent must be in one initialized
  RX block. Public entry and return use Normal mode. Short routines need
  Normal-mode wrappers included in that extent.
- OBJECT: signature=0, size is nonzero; the full extent must fit one R or RW
  block. Zero-filled tails may be exported. Byte alignment is permitted.
  Object layout/constness is part of the library major contract.

The exported runtime address is `origin + value_offset`. Exports do not
create mappings or increase permissions. Function extents may overlap (aliases).

### 4.3 Import entry (24 bytes, WAX or WAL)

Six u32 words: `dependency_index`, `name`, `kind`, `slot_offset`, `size`,
`signature`. Dependency index is zero-based and in range. Match name and kind
against that dependency's exports exactly; missing or mismatched symbols fail.
FUNCTION has size=0 and requires byte-identical signature strings. OBJECT has
signature=0 and nonzero size which MUST equal the exported object's size.
Object layout compatibility remains the library author's major-version promise.

`slot_offset` is image-relative and 4-byte aligned. Its entire 8-byte slot MUST
fit in one RW block. Import entries are sorted by slot_offset; slots cannot
overlap. Repeated imports of the same symbol into distinct slots are allowed.
Both initial slot words MUST be zero (in payload or zero-filled tail).
The loader writes a big-endian descriptor:

```
slot + 0: resolved symbol address (u32)
slot + 4: provider origin B       (u32)
```

For functions the second word supplies the module context (§6). For objects it
identifies the owning module; the first word is the object's address. The OS
patches only declared slots, never instructions. An RW slot remains user
writable after binding; binding is not an integrity boundary within a process.
Libraries share their caller's address space and trust boundary.

## 5. Extension P and the eight-region limit

[Extension P](spec_CPU_priv.md) provides **eight regions per CPU context**,
not eight per image. Region 0 is the WAX stack RW. Sort all loaded blocks from
WAX and the complete WAL graph by runtime address. Merge only exactly adjacent
blocks with equal permissions; never bridge holes or add permission bits.
Assign the resulting ranges to regions 1..N in address order; N MUST be <= 7.
All remaining registers are zero/disabled. Regions must be disjoint.

If no allowed placement fits this budget, reject with REGION_LIMIT. Do not
fall back to supervisor mode, RWX, overlapping grants or whole-RAM access.
There is no per-library region isolation. The allocator MAY pack equal-perm
blocks from separate images adjacently when their reservations allow it.
The OS MUST check grants against its actual ownership map, not merely the
CPU's `0xFFFF0000` reserved-area check. Extension P does not check bus-master
DMA, so the baseline never grants device mappings.

## 6. Public call ABI 1: SRC32 module stack ABI

This is a **new public ABI**, not a claim that existing sol/csc or
`tools/llvm-src32` binaries are compatible. Compilers need explicit v3/PIC
support or adapters. All modules and callbacks use these rules:

- R0=zero; R28=SP, downward stack, 8-byte aligned at call boundaries.
  Arguments are successive 4-byte big-endian words at caller SP. Narrow
  integers are extended to 32 bits; u64/i64 use high word then low word, with
  4-byte argument alignment. Caller rounds the entire argument area to 8 bytes
  and releases it after return. No register arguments or implicit arguments.
- R29=FP and R28 must be restored by the callee. R30 is the **current module
  origin**, preserved by a callee for its caller. R1..R27 and R31 are
  caller-saved. A nested caller must save its return link R31.
- At an inter-module call, save the caller's R30 and read both descriptor words
  into scratch registers before changing R30. Set R30 to descriptor word 1
  and JALR word 0. After return restore
  caller R30 without destroying the result. The called function preserves
  the R30 it received. Internal calls keep R30 unchanged. PIC accesses local
  data/imports using R30 plus image-relative offsets, and uses relative control
  flow or computes targets from R30; no load-time code relocation exists.
- Result: 32-bit/data pointer in R1; 64-bit low in R1, high in R2; fnptr address
  in R1 and origin in R2; void ignores result registers.
  Return with `JR R31` in Normal mode. Every function pointer
  crossing a module boundary is an 8-byte descriptor **by value**, not a raw
  instruction address. Its context must also be retained for callbacks.
- Signatures use ASCII grammar: `fn(` + comma-separated argument types + `)->` + result
  type. Empty arguments are `fn()->...`. Types are `u32`, `i32`, `u64`, `i64`,
  `ptr`, `fnptr`; only result may also be `void`. ptr is a 32-bit data pointer;
  fnptr occupies two words `(address, origin)` as above. No whitespace, floats,
  by-value aggregates or varargs. Narrow integer APIs use u32/i32; aggregates
  use explicit pointers with layouts documented by the library. Signatures
  describe machine calling types, not pointer ownership or semantic safety.

R30 is a module base only in U mode under this ABI. The S bank remains the
kernel's GP/IRQ context. Traps and context switches MUST preserve the full user
context and region registers according to the CPU spec. A kernel service is
reached through SYSCALL under the declared OS ABI, never by importing an S-mode
address. This format assigns no global syscall numbers, including exit.

The WAX entry is `fn()->void`, entered in Normal mode with USP=stack_top,
UFP=0, UGP=base_addr, R1..R27=0 and R31=0. It MUST NOT return: the runtime
requests termination using the declared OS ABI. WALs have no implicit entry;
the WAX runtime calls any explicit setup functions after binding. The OS ABI
defines arguments/environment, exit and fault handling separately.

## 7. Atomic OS launch contract

1. Parse every image and validate the file structure, strings, symbol extents,
   zero slots and ABI constraints. Snapshot the validated bytes so later file
   changes cannot alter the load. Apply documented resource limits before
   allocation (file bytes, counts, graph depth and total resident memory).
2. Resolve the dependency graph and select providers. Reserve private RAM for
   the fixed WAX, all WAL spans and stack. Validate the complete region plan.
3. In S mode copy/zero blocks and stack, and bind all import descriptors.
   Staging cannot overlap any destination. Do not execute library code here.
4. Install syscall/fault handlers and a valid S stack/context. While switching
   contexts with interrupts masked, disable the previous region grants, program
   the complete new grants, initialize the user registers, set EPC=entry,
   STATUS.PU=1, PSM=0 and PIE according to OS policy, then IRET. Enable a region
   only after its START/END have been written. No intermediate context may run.
5. If any prior step fails, free all reservations and expose no partially
   launched process. On exit/fault, quiesce the process, remove its grants and
   free its entire graph; a WAL instance is never freed while that process can
   still execute it. Zero reused allocations before another process sees them.

Logical error categories (OS numeric codes are separate): FORMAT, UNSUPPORTED,
DEPENDENCY_MISSING, ABI_MISMATCH, DEPENDENCY_CONFLICT, DEPENDENCY_CYCLE,
SYMBOL_MISSING, SYMBOL_MISMATCH, ADDRESS_CONFLICT, REGION_LIMIT, RESOURCE_LIMIT.
Diagnostics should identify the requesting image, dependency and symbol where
applicable. Structural validation is not a signature or a trust endorsement.

## 8. Worked binding and conformance cases

APP.WAX: origin `0x100000`, code RX `[0x100000,0x100100)`, data RW
`[0x100100,0x100180)`, entry_offset=0; stack `[0x1FF000,0x200000)`.
It requires `cpt32.console` major 1, min_minor 2 and imports
`write`, FUNCTION, `fn(ptr,u32)->u32`, slot_offset=`0x100`, size=0.
CONSOLE.WAL provides that ID at 1.3, RX block offset 0 size `0x80`, RW block
offset `0x80` size `0x40`, and exports write at offset `0x20`, size `0x10`
with the same signature. All these example blocks are fully initialized.

With B=`0x120000`, binding writes `00 12 00 20 00 12 00 00` to
`0x100100`. Regions are stack RW, APP RX, APP RW, CONSOLE RX, CONSOLE RW;
three unused regions are disabled. Calling write uses PC=`0x120020` and
R30=`0x120000`, then restores the application's R30=`0x100000`.

Required implementation acceptance/rejection cases:

- Accept a dependency-free WAX, a WAL importing another WAL, a repeated import
  into distinct slots, zero-file-size BSS and an object exported from BSS.
- Load the same WAL at two different aligned bases; local data, nested imports
  and a callback descriptor must still resolve to the correct module instance.
- Verify zeroed BSS/stack, Normal-mode entry, R30 restoration, 64-bit ordering
  and process cleanup. Verify ordinary SYSCALL preserves the module context.
- Reject metadata/payload overlap, offset/count/address overflow, malformed or
  interior string offsets, duplicate export names, bad signature syntax,
  invalid table strides, nonzero reserved/slot bytes and unaligned entry.
- Reject absent IDs/symbols, mismatched kind/signature/object size, incompatible
  call/OS/library ABI, major conflicts, cycles, self/duplicate/unused dependencies.
- Reject an import slot outside RW or crossing a block, an export outside its
  required block, entry in BSS, any WX permission, stack/image overlap, bad WAL
  alignment, allocation outside main RAM and a graph needing nine regions.
- Accept a graph needing exactly eight regions including stack; merge adjacent
  equal-permission blocks only. A hole or differing permissions prevents merging.
- Test denied user writes to RX and denied execution of RW/stack, and absence
  of device/kernel grants; never silently launch a rejected image in S mode.
- Existing v2 readers reject v3. A future version-dispatching loader may support
  both, but v2 files do not acquire v3 symbols or ABI semantics implicitly.

The Foundation loader tests cover these cases: format, graph, symbol, region
and placement cases on the host (`foundation/tests/wax/test_wax_loader.py`),
and execution, R30 handling, callbacks, 64-bit ordering, SYSCALL, protection
faults, private instances at two bases and cleanup on src32_testbench
(`foundation/tests/wax/main.c`).
