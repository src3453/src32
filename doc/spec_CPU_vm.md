# SRC32 Extension V — Virtual Memory (draft)

Revision 0.1 (2026-10-22). Status: **implemented in the emulator** (Scalar, Cached,
Cached-relaxed, Pipelined; JIT runs U mode through the Cached interpreter) and
reported by `CPU_FEATURES` bit 9. Tests: `tests/cpu_vm.rs`.

Extension V depends on Extension P (`doc/spec_CPU_priv.md`). Design goal:
**the smallest MMU that lets Foundation give every process its own address
space**. Everything not listed here is out of scope for revision 0.1.

## 1. Summary

| Item | Value |
|---|---|
| CPUID | `CPU_FEATURES` bit 9 (`EXT_V = 0x200`). Software detects V with `CPUID`; there is no other probe. |
| VA / PA | 32 bit / 32 bit |
| Page | 4 KiB, two-level table, VPN 10 + 10 bit, offset 12 bit |
| PTE | 32 bit |
| TLB | 16 entries, fully associative, round-robin replacement, hardware table walk |
| Translated modes | **U only.** S always runs untranslated (VA = PA). |
| Page fault | Precise exception (§6) |
| DMA | Bus masters use physical addresses and are not translated. |
| Not in 0.1 | Huge pages, ASID, swap, demand paging, accessed/dirty updates |

## 2. System registers

New SR numbers (S only, via `MFSR`/`MTSR`; unknown numbers stay illegal):

| SR | Name | Access | Meaning |
|---:|---|---|---|
| `0x40` | `VCTL` | R/W | bit 0 `EN`: translate U-mode accesses. Other bits read as 0. Reset 0. |
| `0x41` | `PTBR` | R/W | Physical address of the level-1 table, 4 KiB aligned (bits 11..0 read as 0). Writing `PTBR` flushes the whole TLB. |
| `0x42` | `VFAULT` | R/W | Faulting virtual address of the last page fault (§6). |
| `0x43` | `TLBFLUSH` | W | Any value: invalidate every TLB entry. Reads as 0. |
| `0x44` | `TLBINV` | W | Invalidate the entry for the page containing the written virtual address. Reads as 0. |

With `VCTL.EN = 0` the CPU behaves exactly as Extension P alone.

## 3. Page table

Level-1 table: 1024 PTEs at `PTBR`. Level-2 table: 1024 PTEs, 4 KiB aligned.
PTEs are 32-bit words stored big-endian, like every other data word (`LD`/`ST`).

```
VA[31:22] = L1 index    VA[21:12] = L2 index    VA[11:0] = offset
```

PTE layout:

| Bits | Name | Meaning |
|---:|---|---|
| 0 | `V` | valid |
| 1 | `R` | readable |
| 2 | `W` | writable |
| 3 | `X` | executable |
| 4 | `U` | accessible from U (always required here, see §4) |
| 5 | `DEV` | Device / non-cacheable page (§7) |
| 7..6 | — | reserved, write 0 (future accessed / dirty bits) |
| 11..8 | `SW` | free for the OS; the CPU ignores them |
| 31..12 | `PPN` | physical page number |

- A level-1 PTE with `V = 1` and `R = W = X = 0` points to a level-2 table at
  `PPN << 12`. A level-1 PTE with any of `R`, `W`, `X` set is a (not supported)
  huge page and faults.
- A level-2 PTE with `V = 1` and at least one of `R`, `W`, `X` set is a leaf.
  `W = 1` requires `R = 1`; otherwise the PTE is invalid.
- Reserved bits that are set make the PTE invalid.

## 4. Translation and permission

When `VCTL.EN = 1` and the CPU is in U mode, every instruction fetch, load and
store goes through the TLB. The Extension P regions (`RGNn_*`) are **not
consulted**. Exception: any access whose virtual or physical address touches
`0xFFFF0000..0xFFFFFFFF` is denied with a protection fault, as in Extension P §7.

An access needs a leaf with `V = 1` and `U = 1`, and `R` (load), `W` (store) or
`X` (fetch). An access that crosses a page boundary is checked per page; if any
byte faults, nothing is read or written.

A TLB miss makes the CPU walk the table in physical memory. The walk is not
subject to protection checks. A bus error during the walk raises a bus error
(CAUSE `0x8002_0000`), not a page fault.

## 5. TLB maintenance

- The TLB holds at most 16 translations, filled by the hardware walk and
  replaced round-robin. Instruction and data accesses share it.
- Software that changes a PTE or a table that the CPU may have cached **must**
  execute `TLBINV` (one page) or `TLBFLUSH` before returning to U.
- `PTBR` writes flush the TLB, so a process switch needs no other action
  (no ASID).
- `IRET`, `SYSCALL` and traps do not touch the TLB.

## 6. Page fault

| CAUSE | Trap | `EPC` | Vector word |
|---|---|---|---|
| `0x8006_000k` | Page fault in U | faulting instruction | `0xFFFF0008` (shared with bus error) |

`k`: bit 0 read, bit 1 write, bit 2 instruction fetch. Bit 3 distinguishes a
missing translation (0: `V = 0`, bad PTE, no leaf) from a permission failure
(1: leaf present but `R`/`W`/`X`/`U` denies the access). `VFAULT` receives the
faulting virtual address (for a page-crossing access, the first faulting byte).

Faults are precise (Extension P §6.1): the faulting instruction has no effect.
Trap entry otherwise follows Extension P §6.

## 7. Device pages and DMA

- A `DEV = 1` page is accessed exactly once per instruction access, in order,
  and is never cached or merged. Cached CPU kinds bypass their cache for it.
  Instruction fetch from a `DEV` page is allowed only if `X` is set.
- DMAC, BMC, IDC and VPU masters use physical addresses and ignore the page
  table. The OS must translate and validate buffers before programming them.

## 8. Implementation notes (emulator, planned)

- All CPU kinds share the translation function next to the region check in
  `src/cpu_privilege.rs`; the TLB state belongs with the privilege state.
- Pipeline: `MTSR` to a V register flushes the pipeline like other SR writes,
  so a translation never changes under an instruction in flight.
- JIT: native blocks run only in S mode (unchanged); U mode uses the Cached
  interpreter and therefore the translation path.
- The TLB is an architectural resource in the sense that only flush/invalidate
  ordering is specified; an implementation may cache more than 16 entries as
  long as software cannot observe it.

## 9. Compatibility

A CPU without bit 9 raises an illegal-instruction exception for the SR
numbers `0x40..0x44`. Foundation selects its memory backend from this bit
(see `doc/foundation_vm_plan.md`).
