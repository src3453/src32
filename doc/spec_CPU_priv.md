# SRC32 Extension P (uppercase) — Privilege Levels and System Calls

Revision 1.0 (2026-10-21). Status: implemented in every CPU kind
(`src/cpu_privilege.rs`, `src/cpu.rs`, `src/cpu_pipeline.rs`).

Extension P depends on Extension I. Case is significant: lowercase `p`
(Pipelining, `doc/spec_CPU_p.md`) is a different extension.

Design goal: **clarity first**. There are two modes, one trap entry rule, one
return rule, and three new instructions.

## 1. Summary

| Item | Value |
|---|---|
| Modes | Supervisor (S) and User (U). Reset enters S. |
| CPUID | `CPU_FEATURES` bit 8 (`EXT_P = 0x100`). Scalar/Cached/JIT report `0x13F`, Pipeline `0x1BF`. |
| New instructions | `SYSCALL imm16` (`0x23`), `MFSR rd, sr` (`0x24`), `MTSR sr, rs` (`0x25`) |
| Privileged instructions | `HALT`, `IRET`, `MFSR`, `MTSR` |
| Banked registers | `R28` (SP), `R29` (FP), `R30` (GP) have separate S and U copies |
| User memory | Default deny. Eight S-programmed regions grant R/W/X. CPU-reserved area is never accessible from U. |
| Exceptions | Illegal instruction, bus error, SYSCALL, privileged instruction, protection fault |

A program that never leaves S behaves as before, except for the changes listed
in §11.

## 2. Modes

- **S (Supervisor)**: every instruction and address is allowed. Reset, IRQ
  entry and every exception enter S.
- **U (User)**: `HALT`, `IRET`, `MFSR` and `MTSR` raise a privileged-instruction
  exception. Every instruction fetch, load and store is checked against the
  protection regions (§7).

The only ways to change mode are trap entry (U→S) and `IRET` (restores the
saved mode). To start a user program, S sets `EPC` and `STATUS.PU`, then runs
`IRET` (§10.1).

### 2.1 Boot policy

The separately specified [WAX/WAL v3](spec_WAX_v3.md) user-module format
(not implemented yet) budgets these same eight regions across the executable,
all libraries and stack. It adds no CPU registers or supervisor-call bypass.

- Reset always enters S, so existing programs, raw `.bin` images and
  unprotected `.wax` images run exactly as before.
- User mode is opt-in per executable: a `.wax` with the `PROTECTED` flag
  (`doc/spec_WAX.md`) declares its stack and per-block R/W/X permissions.
- The BIOS WAX loader performs the S→U hand-off for such an image. Host
  direct loading never enters U and refuses protected images.

## 3. Instructions

All three use the Normal-mode immediate format `[op:6][rd:5][rs1:5][imm16:16]`.
There are no Short-mode forms. Leave a Short-mode routine with `S.RET` first.

### 3.1 `SYSCALL imm16` — opcode `0x23`

- `imm16[7:0]`: system call number, `0..255`.
- `imm16[15:8]`: context byte. Reserved for the OS. Use 0 when not needed; the
  CPU only copies it.
- `rd` and `rs1` are ignored and should be 0.
- Allowed in both S and U. Registers are unchanged.
- Effect: trap with `CAUSE = 0x8003_0000 | imm16`, `EPC = address of the next
  instruction`, vector word `0xFFFF000C`.

Assembler: `SYSCALL n` (n = 0..255) or `SYSCALL n, ctx` (ctx = 0..255).

### 3.2 `MFSR rd, sr` — opcode `0x24` (S only)

`rd ← SR[imm16]`. `rs1` is ignored. An unknown `sr` number is an illegal
instruction.

### 3.3 `MTSR sr, rs` — opcode `0x25` (S only)

`SR[imm16] ← rs`. The source register is in the `rs1` field; `rd` is ignored.
An unknown `sr` number is an illegal instruction. Read-only bits keep their
value. `MTSR` takes effect before the next instruction.

Assembler: `MFSR R1, STATUS`, `MTSR EPC, R2`. The SR operand accepts a name
from §4 or a number.

## 4. System registers (SR)

| SR | Name | Access | Meaning |
|---:|---|---|---|
| `0x00` | `STATUS` | R/W | Mode and interrupt state (§4.1) |
| `0x01` | `EPC` | R/W | Return address for `IRET` |
| `0x02` | `CAUSE` | R/W | Reason for the last trap (§5) |
| `0x08` | `USP` | R/W | User bank `R28` |
| `0x09` | `UFP` | R/W | User bank `R29` |
| `0x0A` | `UGP` | R/W | User bank `R30` |
| `0x10 + 4n` | `RGNn_START` | R/W | Region `n` start address (inclusive), `n = 0..7` |
| `0x11 + 4n` | `RGNn_END` | R/W | Region `n` end address (exclusive) |
| `0x12 + 4n` | `RGNn_FLAGS` | R/W | Region `n` permission bits (§7) |

Every other number, including `0x13 + 4n`, is unknown.

`USP`/`UFP`/`UGP` always name the **user** copies. S code uses them to set up
or save a user context. In U mode, `R28`–`R30` are the user copies themselves.

### 4.1 STATUS

| Bit | Name | Meaning |
|---:|---|---|
| 0 | `IE` | Interrupt enable |
| 1 | `PIE` | `IE` before the last trap |
| 2 | `U` | Current mode: 0 = S, 1 = U. **Read-only.** |
| 3 | `PU` | Mode before the last trap |
| 4 | `PSM` | Instruction mode before the last trap: 0 = Normal, 1 = Short |
| 31..5 | — | Read as 0, writes ignored |

Reset value: `IE=1, PIE=1, U=0, PU=0, PSM=0` (`0x03`).

`MTSR STATUS` with `IE=1` enables interrupts. S code can use this to allow
nested interrupts after it has saved `EPC`, `CAUSE` and `STATUS`.

The CPU-state MMIO word `0xFFFF028C` shows the same STATUS value. As before,
guest stores to CPU-state MMIO are overwritten at commit. Use `MTSR`. A host
debugger write to that word can change `IE`, `PIE`, `PU` and `PSM`; `U` is
read-only there too.

## 5. CAUSE

| CAUSE | Trap | `EPC` | Vector word |
|---|---|---|---|
| `0x0000_000n` | IRQ `n` (0..15) | next instruction | IRQ slot `0xFFFF0100 + 4n` (instruction) |
| `0x8001_0000` | Illegal instruction | faulting instruction | `0xFFFF0004` |
| `0x8002_0000` | Bus error | faulting instruction | `0xFFFF0008` |
| `0x8003_xxxx` | `SYSCALL` (`xxxx` = imm16) | next instruction | `0xFFFF000C` |
| `0x8004_0000` | Privileged instruction in U | faulting instruction | `0xFFFF0004` |
| `0x8005_000k` | Protection fault in U | faulting instruction | `0xFFFF0008` |

Layout:

- bit 31: 0 = IRQ, 1 = exception.
- bits 23..16: exception code (1..5).
- bits 15..0: detail. SYSCALL: `imm16` (bits 7..0 = number, 15..8 = context).
  Protection fault: `k` = 1 read, 2 write, 4 instruction fetch.

The IRQ values are unchanged, so existing dispatchers that compare CAUSE with
0..15 keep working.

Instruction problems share the illegal-instruction vector; access problems
share the bus-error vector. CAUSE tells them apart.

## 6. Trap entry and IRET

Every trap (IRQ or exception) does exactly this:

```
EPC        = see §5
CAUSE      = see §5
STATUS.PIE = STATUS.IE
STATUS.IE  = 0
STATUS.PU  = STATUS.U
STATUS.PSM = (INSTR_MODE == Short)
if STATUS.U == 1: swap R28..R30 to the S bank
STATUS.U   = 0
INSTR_MODE = Normal
PC         = IRQ slot address, or the word stored at the exception vector
```

If an exception vector word still holds its own address (the reset default),
the CPU halts at that vector instead of looping. The default vector words are
`0xFFFF0004`, `0xFFFF0008` and `0xFFFF000C`.

A pending IRQ latch is kept across an exception. It is taken once `IE` is 1
again.

`IRET` (S only):

```
PC         = EPC
STATUS.IE  = STATUS.PIE
INSTR_MODE = STATUS.PSM ? Short : Normal
if STATUS.PU == 1: swap R28..R30 to the U bank
STATUS.U   = STATUS.PU
```

`PIE`, `PU` and `PSM` keep their values after `IRET`.

### 6.1 Precision

Exceptions are precise. The faulting instruction has no effect: a denied store
writes nothing, a denied load does not read the bus, and `SYSCALL` changes
nothing except the trap state. Older instructions have completed; younger
instructions have not started.

## 7. User-mode memory protection

Checks apply only in U mode. S mode is not checked.

1. Any access whose bytes touch `0xFFFF0000..0xFFFFFFFF` (CPU vectors, IRQC,
   CPU state) is denied.
2. Otherwise **every byte** of the access must lie in at least one *active*
   region that grants the needed permission.

| Access | Permission |
|---|---|
| Load (`LD`, `LDH`, `LDB`, `S.LD`) | R (`FLAGS` bit 0) |
| Store (`ST`, `STH`, `STB`, `S.ST`) | W (`FLAGS` bit 1) |
| Instruction fetch (4 bytes Normal, 2 bytes Short) | X (`FLAGS` bit 2) |

A region is active when `FLAGS & 7 != 0` and `START < END`. It covers
`START <= addr < END`. Regions may overlap; permissions are combined (any
region granting the bit allows the byte). `FLAGS` bits 31..3 read as 0.

At reset every region is zero, so U mode can access nothing until S grants it.

A denied access raises a protection fault (§5) before the bus is touched.

Granting a device window (for example VDP `0x80000000..0x80010000`) lets a
game drive that device directly. Do not grant DMAC, BMC, IDC or expansion
cards unless you trust the user code: DMA is a bus master and is not checked.

## 8. Banked registers

`R28`, `R29` and `R30` exist twice. The CPU uses the S copy in S mode and the
U copy in U mode. Every other register, including `R31` (LR), is shared.

This keeps the existing IRQ vector style (`JR R30` in the slot) and the sol
dispatcher safe. On a trap from U, `R30` and `R28` are the kernel's values, so
user code cannot redirect the handler or its stack.

The CPU-state MMIO view `0xFFFF0200..` and the debugger show the bank of the
current mode.

## 9. Implementation notes (emulator)

- All CPU kinds share `src/cpu_privilege.rs`: SR numbers, CAUSE values, STATUS
  bits, region checks and bank swapping.
- Pipeline: `SYSCALL`, `MTSR`, `IRET` and all exceptions are handled at commit
  and flush the pipeline, then fetch restarts at the new PC. Mode, banks and
  regions therefore never change under an instruction in flight.
- JIT: native blocks run only in S mode. U mode uses the Cached interpreter.
  `SYSCALL`, `MFSR` and `MTSR` always end a block.
- Spin-loop fast-forward compares the privilege state too.
- C toolchain: `tools/llvm-src32` provides `cpt32_syscall`/`cpt32_mfsr`/
  `cpt32_mtsr`/`cpt32_run_user`/`cpt32_return_to_kernel` intrinsics, a
  `syscall_dispatch` hook and `--protected` WAX output; see its README and
  `examples/syscall-user-c`.

## 10. Examples

### 10.1 Starting a user program

```
    ; region 0: user code+data at 0x00100000..0x00200000, R/W/X
    LDIH R1, 0x0010
    LDIL R1, 0x0000
    MTSR RGN0_START, R1
    LDIH R1, 0x0020
    LDIL R1, 0x0000
    MTSR RGN0_END, R1
    ADDI R1, R0, 7
    MTSR RGN0_FLAGS, R1
    ; user stack top
    LDIH R1, 0x0020
    LDIL R1, 0x0000
    MTSR USP, R1
    ; enter U at user_main with interrupts on
    LDIH R1, user_main
    LDIL R1, user_main
    MTSR EPC, R1
    ADDI R1, R0, 0x0B      ; PU=1, PIE=1, IE=1
    MTSR STATUS, R1
    IRET
```

### 10.2 SYSCALL handler skeleton

```
    ; 0xFFFF000C holds syscall_entry
syscall_entry:
    ; R28 is the S stack here, even when called from U
    ADDI R28, R28, -8
    ST [R28 + 0], R13
    ST [R28 + 4], R14
    MFSR R13, CAUSE
    ADDI R14, R0, 0xFF
    AND R13, R13, R14      ; R13 = call number (CAUSE bits 7..0)
    ...                    ; dispatch on R13
    LD R14, [R28 + 4]
    LD R13, [R28 + 0]
    ADDI R28, R28, 8
    IRET                   ; returns after the SYSCALL
```

## 11. Compatibility with earlier revisions

- Bus error `CAUSE` changes from `2` to `0x8002_0000`. IRQ2 keeps `CAUSE = 2`,
  so the two no longer collide.
- An illegal instruction raises an exception instead of stopping the emulator
  with a host panic.
- `IRET` restores `IE` from `PIE` instead of always setting it to 1. For IRQ
  handlers this is identical, because an IRQ is only taken while `IE = 1`.
- An exception keeps a pending IRQ latch. (Before, bus error entry cleared it.)
- Pipeline: `IRET` now redirects at commit instead of EX, so it costs a few
  more cycles.
- `0xFFFF0104` is IRQ1's slot. An earlier draft placed SYSCALL there; the
  SYSCALL vector is `0xFFFF000C`.
