# 3HS88PWN4 reference model (C++17)

Integer-only C++ model of the full 3HS88PWN4: eight harmonic channels (the
3HS816 core in [`../3hs816`](../3hs816)), four PCM/WT/noise channels with
their filters, the 3-band EQ and the compressor. The contract is the
"3HS88PWN4（整数モデル）" section of
[`doc/spec_3HS816.md`](../../doc/spec_3HS816.md); the register map is the
original chip's.

| Path | Contents |
|---|---|
| `include/hs88/engine.hpp` | `Engine`: channels, PCM RAM, mixer, effects, `render()` |
| `include/hs88/pcm_channel.hpp` | `PcmChannel`: cursor, modes, PCM addressing, filter, FIFO |
| `include/hs88/master_effects.hpp` | EQ and compressor on the Q8 DAC-unit bus |
| `include/hs88/biquad.hpp` | fixed-point direct-form-I biquad |
| `include/hs88/registers.hpp` | `RegisterInterface`: PCM RAM and register decode |
| `src/generated_tables.hpp` | noise, filter trig, EQ and detector ROMs from `tools/gen_tables.py` |

Furnace's S3HS and 3HS816 use copies of `include/` and `src/` of both models
(`src/engine/platform/sound/3hs/` there). Change the model here, run the
tests, then copy it again.

## Build and test

```bash
cmake -S chips/3hs88pwn4 -B build/hs88 -DHS816_LEGACY_SOURCE=/path/to/s3hs88pwn4/sound.cpp
cmake --build build/hs88 --config Release
ctest --test-dir build/hs88 -C Release --output-on-failure
```

The legacy float source is no longer in Furnace's tree; extract it from
Furnace commit `0889f291d`:

```bash
git -C /path/to/furnace archive 0889f291d src/engine/platform/sound/s3hs88pwn4 | tar -x -C /tmp/hs88-legacy
```

The CMake project pulls in `../3hs816` itself.
