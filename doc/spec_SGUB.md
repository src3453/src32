# SGUB v1 music stream

SGUB is a register stream played at 60 Hz by the host VSync loop or
`sol/cpt32/sgu_music_driver.sol`. `src/music/stream.rs::StreamPlayer` validates
all data before playback. Maximum file size is `0x00E00000` bytes (14 MiB),
allowing a stream loaded at main RAM address `0x00200000` to fit in 16 MiB RAM.

All multi-byte fields are **big endian**.

This document specifies v1, retained for Music Editor output and existing
files. The current Furnace ROM exporter writes pooled v2, specified in
`doc/spec_SGUB_v2.md`. Both versions use the current guest driver; rebuild it
with `solc.py compile -O speed` after updating its source.

| Offset | Size | Meaning |
|---|---|---|
| 0 | 4 | ASCII `SGUB` |
| 4 | 1 | Version: 1 |
| 5 | 1 | Voice count: 16 (two eight-channel SGUs) |
| 6 | 1 | Flags: bit 0 repeats the entire frame sequence; other bits zero |
| 7 | 1 | Reserved: zero |
| 8 | 4 | Frame count, at least 1 |
| 12 | 4 | Initial record count |
| 16 | 4 | Exact total byte size, including header |
| 20 | 5 × initial count | Initialization records |
| following | variable | Frames: u16 record count followed by 5-byte records |

A record is `(target: u8, address: u24, value: u8)`. Targets 0 and 1 write
SGU0/SGU1 registers at offsets `0x000..0x8FF`; target 2 writes shared PCM RAM
at offsets `0x00000..0xFFFFF`. Targets and addresses outside these ranges are
invalid. Records are applied in order; duplicate phase-reset and PCM-control
writes have side effects and must not be deduplicated. No bytes may follow the
last declared frame.

The first VSync applies initialization then frame zero. Each subsequent VSync
applies exactly one frame. After the final frame, the next VSync either starts
frame zero again or silences channel volume and PCM control on both SGUs.
Initialization records are not replayed on repetition. There is no stored
internal loop point or sub-frame timestamp.

## Furnace fork producer

The local Furnace fork at `../furnace` provides **File > Export > ROM >
CPT32 SGUB (S3W2)** and CLI `furnace -romout song.sgub song.fur`.
It supports songs with one or two S3W2 chips, accumulates tick durations at
48 kHz and quantizes writes to their containing 60 Hz frame. Sub-frame effects
lose their spacing. An optional `-romconf loop=true` repeats from the beginning;
frame zero resets both chips to avoid stale register and wavetable state.

Furnace's signed PCM bytes are converted by adding 128 modulo 256. Its two
independent chip RAM layouts are merged when compatible. Otherwise bank 1 is
packed after bank 0 with a silent end guard and its start/end/loop register
writes are relocated, propagating carries across address bytes. Export rejects
banks plus guards exceeding the shared 1 MiB RAM. Individual samples are not
deduplicated across conflicting banks. Furnace S3W2 is a stub,
not the normative CPT32 implementation: its Slew IDs 3/4 are translated to
CPT32 IDs 5/6, retaining CPT32 Hard Sync/Window at 3/4. Reserved Furnace
modes 5..31 become None with a warning, matching the stub's no-op behavior.
Host mixer gains/routing are not register data. Historical v1 exports included
a full 1 MiB PCM image as initialization (about 5 MiB encoded). Current v2
exports store used PCM bytes in a pool and initialize silence with FILL.

Furnace pitch processing uses Hz for wavetable/noise, and a PCM register unit
represents 32 samples/second. PCM notes are scaled using the sample's C-4 rate;
Pitch macros do not apply a PCM divisor to wavetable frequency.

Example playback with an existing guest:

```sh
cargo run --release --bin cpt32 -- dos.wax --music-stream song.sgub
```
