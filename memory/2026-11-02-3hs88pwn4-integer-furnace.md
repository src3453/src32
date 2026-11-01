# 3HS88PWN4 integer model and Furnace S3HS / 3HS816

`chips/3hs88pwn4/` is the integer-only 3HS88PWN4 (8 HS + 4 PCM/WT, filters, EQ, compressor) on top of the 3HS816 core; contract in the "3HS88PWN4（整数モデル）" section of `doc/spec_3HS816.md`. Furnace's S3HS and new 3HS816 (branch `3hs816` in the `furnace-3hs816` worktree) use copies of both models. See also [[2026-11-02-3hs816-fixed-point-model]].

- **Source of truth is here.** Furnace's `src/engine/platform/sound/3hs/` is a plain copy of `include/` and `src/`; edit here, run both chips' tests, copy back. `diff -r` the trees before committing either side.
- **The float reference is gone from Furnace's tree.** For the legacy comparisons extract it: `git -C <furnace> archive 0889f291d src/engine/platform/sound/s3hs88pwn4 | tar -x -C <dir>` (sound.cpp SHA-256 `518671b1…`).
- **WT/PCM pitch is `floor(192000/P)` Hz on purpose.** The reference added the float frequency to an integer accumulator, dropping the fraction each sample. Using the exact `192000/P` would put every WT channel slightly sharp versus the original.
- **The PCM loop rule is the reference's odd one** (`start + i mod (end − loop)`, loop start not added). Furnace writes the loop end as `end`, so it works for its data; don't "fix" it without changing the Furnace dispatch too.
- `resetGate()` / `wtSync()` became register writes (gate=1, WT `+8`), matching the Rust card spec; SGUB exports now contain them.
- The whole render verification is real-engine: `python test/3hs/render_check.py build/Release/furnace.exe --reference <old furnace.exe>` in the Furnace worktree. Pitch must use interpolated zero crossings (plain counting misread 440 Hz as 330). Furnace's output removes DC, so DC user waves can't be checked. A/B against the float core can be bit-identical after 44.1 kHz resampling; that is not evidence the reference didn't run (the check requires the reference to be audible instead).
- Gotchas hit while building: a braced `{a,b,c,d,e}` argument picked the `int32_t[5]` overload and recursed forever (MSVC C4717 caught it); Furnace's checkout is CRLF, so exact-match patch scripts must normalize line endings; heredocs turned `\0` in Python sources into real NUL bytes.
- Furnace's `test/sgub/smoke.py` already fails at HEAD (expects the v1 header; writer emits v2/v3). Not caused by this work.
