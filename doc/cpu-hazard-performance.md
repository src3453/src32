# Pipeline CPU hazard-check performance

2026-10-18: `PipelineCpu` only decodes hazard sources when EX contains a load.
Its temporary source list is a two-element stack array, replacing a heap-allocated
`Vec`. Unused entries contain R0; the hazard predicate must continue excluding R0.
Instruction timing, forwarding, IRQ handling and byte-by-byte bus accesses are unchanged.

Reproduce with `cargo run --release --bin cpu_bench -- 100000 15` on the old and
new revisions, sequentially. The benchmark checks architectural state against Scalar
and reports cycle, stall and flush counts. One sample executes 900,006 instructions.

| Sequential comparison | Before | After | Time reduction |
| --- | ---: | ---: | ---: |
| Pipeline median, first pair | 338.094 ms | 298.639 ms | 11.7% |
| Pipeline median, second pair | 340.437 ms | 304.716 ms | 10.5% |

These Windows-host synthetic ALU/MUL/load/store results are workload-specific.
Background load varied (Scalar medians also moved); GUI and real guest speedups
have not been measured. Pipeline counters remained 1,400,008 cycles, 300,000 stalls,
and 99,999 flushes. Fetch/bus and Device/IRQ remain further profiling candidates;
combining memory accesses requires preserving DMA arbitration and MMIO side effects.

Validation: the hazard-source regression compares against ID decoding for every
16-bit encoding, all normal opcodes with rotating GPR fields, and every potential
load destination including R0. CPU integration tests cover exact stalls, forwarding,
interrupts, mode transitions and Scalar equivalence.
