# 3HS816 reference model (C++17)

Integer-only C++ model of the 3HS816 sound chip: 16 channels of the
3HS88PWN4 harmonic (8-operator) synthesizer, without PCM or effects. It is the
reference for an RTL implementation. The contract is
[`doc/spec_3HS816.md`](../../doc/spec_3HS816.md).

## Layout

| Path | Contents |
|---|---|
| `include/hs816/engine.hpp` | `Engine`: 16 channels, mixer, `render()` to int16 stereo at 48 kHz |
| `include/hs816/channel.hpp` | `Channel`: operators, phases, feedback, algorithm routing |
| `include/hs816/envelope.hpp` | ADSR with half-sample timing and the 2^(-x) ROM |
| `include/hs816/wavetables.hpp` | built-in wave ROM and the 4 user waves |
| `include/hs816/registers.hpp` | `RegisterInterface`: byte bus decode into `Engine` calls |
| `src/generated_tables.hpp` | ROM data written by `tools/gen_tables.py`; do not edit |

The engine has no notion of addresses; drive it through `RegisterInterface`
(as a bus device) or call the engine directly (as a tracker would).

```cpp
hs816::Engine engine;
hs816::RegisterInterface regs(engine);
regs.write(0x00, 0x01); regs.write(0x01, 0xB8);  // ch0 OP1 = 440 Hz
regs.write(0x10, 255);                          // OP1 volume
regs.write(0x22, 255);                          // OP1 sustain
regs.write(0x1F, 128);                          // no feedback
regs.write(0x1E, 1);                            // key on
engine.render(left, right, frames);
```

No float or double may appear in `include/` or `src/`; anything that needs
transcendental math belongs in `tools/gen_tables.py`. After changing it, run
`python tools/gen_tables.py` and commit the regenerated header.

## Build and test

```bash
cmake -S chips/3hs816 -B build/hs816 -DHS816_LEGACY_SOURCE=/path/to/furnace/src/engine/platform/sound/s3hs88pwn4/sound.cpp
cmake --build build/hs816 --config Release
ctest --test-dir build/hs816 -C Release --output-on-failure
```

`HS816_LEGACY_SOURCE` is optional. Without it only `hs816_tests` and
`hs816_sources` run; with it, `hs816_legacy_compare` also checks the model
against the 3HS88PWN4 reference (it reads the Furnace source, never modifies
it). The compare tiers are described at the end of the spec.
