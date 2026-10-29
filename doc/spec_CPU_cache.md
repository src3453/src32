# SRC32 Extension C — L1 Cache

Revision 0.1 (2026-10-26). Status: implemented in the pipeline emulator
(`src/cpu_cache.rs`) and the FPGA RTL (`src32-sv/rtl/src32_l1.sv`).

## 1. Position in the extension scheme

`C` is a micro-architectural extension: **no ISA change and no software
visible data**. Programs written without Extension C run unchanged, with no
cache maintenance.

- CPUID bit 10 (`EXT_C = 0x400`). The pipeline CPU reports
  `CPU_FEATURES = 0x7BF`. Scalar, Cached and JIT interpreters do not
  implement `C` and keep `0x33F`.
- Uppercase `C` (cache) is unrelated to lowercase extensions `p`/`v`.

## 2. Organisation

Split Harvard L1, both caches physically indexed and tagged (the address
after Extension V translation, so there are no aliasing problems and no flush
on `MTSR`/TLB changes).

| | Instruction | Data |
|---|---|---|
| Size | 8 KiB | 8 KiB |
| Line | 32 bytes (8 words) | 32 bytes |
| Ways | 1 (direct mapped) | 1 |
| Write policy | read-only | write-through, no write-allocate |
| Fill | whole line, burst | whole line, burst |

Only **Main RAM** is cacheable (on the FPGA SoC: the SDRAM windows, including
the RAM disk window). Everything else — device MMIO (`0x8000_0000`+), the boot
ROM, CPU-reserved MMIO — bypasses the caches. An access is classified by its
first byte; a line fill never crosses a 32-byte boundary.

## 3. Coherence (the architectural contract)

While a cache is enabled it never returns data different from what an
uncached access would:

- A CPU store updates the data line if it hits (write-through) and always
  writes memory. A store that misses does not allocate.
- A CPU store also invalidates the matching instruction-cache line, so
  self-modifying code and loaders need no flush. (The pipeline already
  discards younger fetches after a store; the I-cache snoop makes the same
  guarantee for lines fetched earlier.)
- Other bus masters (DMA in the emulator) invalidate overlapping lines.
  The FPGA SoC has none. In the emulator the model is tag-only, so it cannot
  return stale data; DMA writes are not snooped and may only skew the counters.
- Setting or clearing an enable bit invalidates that cache (a disabled cache
  does not snoop stores).

## 4. Registers (CPU-reserved MMIO, supervisor only)

`0xFFFF0300..0xFFFF033F`, big-endian words; user mode cannot access this
window (see `spec_CPU_priv.md`). Unlisted offsets read 0.

| Offset | Name | Access | Description |
|---|---|---|---|
| `0x00` | `CACHE_INFO` | RO | `[3:0]` log2 line bytes (5), `[7:4]` log2 ways (0), `[15:8]` log2 I-cache bytes (13), `[23:16]` log2 D-cache bytes (13) |
| `0x04` | `CACHE_CTRL` | RW | bit0 `I_EN`, bit1 `D_EN` (both 1 after reset); bit8 `INV_I`, bit9 `INV_D` invalidate all (write 1, read 0); bit10 `CLR_STATS` |
| `0x10` | `I_HITS` | RO | instruction fetch hits (32-bit wrapping) |
| `0x14` | `I_MISSES` | RO | instruction fetch misses |
| `0x18` | `D_HITS` | RO | data load hits |
| `0x1C` | `D_MISSES` | RO | data load misses |

Stores are not counted. Counters include speculative pipeline fetches (for
example the fetches that run past a `HALT`).

## 5. Timing

Hit and miss latency are implementation defined; software must not depend
on them.

- **FPGA**: a hit takes the normal one-cycle bus handshake. A miss fills a
  line with one SDRAM burst (about 20 clocks at 48 MHz, against about 12 clocks
  for every uncached word). Write-through stores keep the SDRAM single-word
  write latency.
- **Emulator**: by default the cache is a statistics model and charges no
  extra cycles, so existing cycle expectations hold. `set_cache_miss_penalty(n)`
  stalls the pipeline `n` cycles per miss for performance studies.

## 6. Reset

Both caches are enabled, empty, and the counters are zero.
