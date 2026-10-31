# SRC32 Extension p (lowercase) — Pipelining

Revision 0.1 (2026-10-15). Status: implemented (`src/cpu_pipeline.rs`).

## 1. Position in the extension scheme

Case is significant. Uppercase `P` (Protection/Privilege Levels) and
lowercase `p` (Pipelining) are different extensions:

- `P`: Protection/Privilege Levels (depends on Extension I). Implemented; see
  `doc/spec_CPU_priv.md`. CPUID bit 8.
- `p`: Pipelining. Micro-architecture only; **no ISA change**. Implemented.

`p` is advertised by CPUID bit 7:

- `EXT_P_SMALL = 0x80` (the name keeps the lowercase-`p` meaning; the
  constant follows Rust upper-case convention).
- A pipelined CPU reports `CPU_FEATURES = 0x1BF`
  (`Base|A|L|M|S|I|P` plus `p`). The scalar reference CPU reports `0x13F`.
- Extension P changes of mode, banks or protection regions (`SYSCALL`, `MTSR`,
  `IRET`, every exception) are handled at commit and flush the pipeline, so no
  in-flight instruction sees a stale privilege state. `IRET` therefore
  redirects at commit, not EX.
- Bit 6 stays reserved for Extension F (defined, not implemented).

## 2. Micro-architecture (what `p` guarantees)

Five-stage, in-order pipeline: `IF / ID / EX / MEM / WB`.

- All 32-bit Normal and 16-bit Short instructions flow through the same
  stages. Fetch width follows the fetch mode; mode switches
  (`JMPS`/`JALS`/`JRS`/`JALRS`, `S.RET`) are branches resolved in EX.
- Every stage carries `valid` plus the instruction's own PC and mode, so
  commit stays precise and flushes never confuse bubbles with real NOPs.
- `R0` reads zero and ignores writes at commit; it is excluded from
  forwarding and hazard checks.
- Ideal throughput is one instruction per cycle after fill; stalls, flushes
  and multi-cycle ops make real IPC < 1.

### 2.1 Forwarding

- EX inputs select from `EX/MEM`, then `MEM/WB`, then the register file
  (live read at EX, so producers that committed after ID are still seen).
- A load sitting in `EX/MEM` is never forwarded (its data arrives in MEM);
  that case is covered by the load-use stall below.

### 2.2 Load-use hazard: 1-cycle stall

`LD` followed by an instruction reading the loaded register stalls fetch
and ID for one cycle and injects a bubble into `ID/EX` while the load still
advances `EX -> MEM`. The waiter is re-decoded afterwards and forwards the
value from `MEM/WB`.

### 2.3 Branches: static prediction and EX resolution

`BEQ`/`BNE`/`JMP`/`JAL`/`JR` (and Short-mode counterparts) resolve in EX.
Conditional branches (`BEQ`, `BNE`, `S.BZ`, `S.BNZ`) use backward-taken,
forward-not-taken prediction in ID. EX checks the actual result: correct
predictions continue without a redirect; a misprediction redirects and flushes
younger instructions. Unconditional branches still redirect in EX.
`JAL`/`JALR` link `R31` with the fall-through PC at commit and the
link value participates in forwarding.

### 2.4 Multi-cycle ALU ops

- `MUL`/`MULH` occupy EX for 3 ticks, `DIV`/`DIVU`/`MOD` for 16 ticks
  (same latencies as the scalar reference). The op waits in `ID/EX` while
  the front end holds and `MEM`/`WB` drain; results and edge semantics
  (divide by zero → 0, `INT_MIN / -1` → `INT_MIN`, `INT_MIN % -1` → 0)
  match the scalar CPU exactly.

### 2.5 Precise interrupts, HALT

- IRQs are sampled only at instruction-commit boundaries. `EPC` holds the
  address after the last committed instruction (`retire_pc`) and the saved
  mode; `IRET` restores both.
- `HALT` takes effect at commit and discards over-fetched fall-through
  instructions (the scalar CPU never fetched them). Wake-up via IRQ refetches
  from `EPC`, so post-`HALT` code runs exactly once, after the wake-up.

### 2.6 Memory and bus

- Each instruction's `MEM` access executes exactly once per commit path.
  Stalls hold fetch without re-issuing bus requests; `EX/MEM` drains to a
  bubble instead of re-executing `MEM`, so MMIO side effects fire once.
- Single-bus rule: `MEM` has priority over `IF` within a tick
  (`MEM` is serviced before the next fetch). DMA continues to be drained by
  the existing bus arbitration ahead of CPU accesses.
- Future wait-state/bus-error support must hold `IF` (never duplicate `MEM`)
  and bind the error to the faulting instruction's PC.

### 2.7 Counters

`cycles`, `instructions_executed` (WB commits), `stall_cycles`
(load-use + MUL/DIV holds), `flush_events` (EX redirects, including branch
mispredictions and unconditional branches).

## 3. Reference model and tests

- The scalar `Cpu` (`src/cpu.rs`) is the reference. Same program, same
  initial memory → same registers, same architectural PC, same memory.
  `tests/cpu_pipeline.rs::differential_mixed_program_matches_scalar` checks a
  mixed program; directed tests assert the exact single-cycle load-use stall,
  branch flush behavior, store forwarding, and exact stalls plus dependent
  results for `MUL`, `MULH`, `DIV`, `DIVU`, and `MOD`.
- The emulator boots on `PipelineCpu` by default. `src/cpu_select.rs` exposes
  `--cpu scalar|cached|pipeline` and `cpu.microarchitecture` through `AnyCpu`;
  Cached mode reuses scalar instruction semantics in a dedicated
  single-instruction-per-tick interpreter with a write-coherent decode cache;
  it does not advertise Extension p.
  Cache hits skip RAM instruction-fetch bus access; pipeline remains the
  default, and CPU mode choice should be based on measurements for the target
  workload and host.

## 4. Files

- `src/cpu_pipeline.rs` — implementation.
- `tests/cpu_pipeline.rs` — regression suite (15 tests).
- `doc/spec_CPU.md` — ISA source of truth (unchanged semantics).
- `src/bin/cpu_bench.rs` — three-way Scalar/Pipeline/Cached comparison; the
  loop workload checks architectural state and reports median wall time,
  cache hit/miss counts, and Cached speedup against both reference modes.
