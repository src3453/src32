# 3HS chips: frequency registers are exact Hz (period quantization removed)

The 3HS88PWN4 family used to quantize every frequency register to a period of a 192 kHz clock: `P = floor(192000/F)`, played at `192000/P` Hz (and the float WT/PCM path even at `floor(192000/P)`). By the user's decision this is removed from the specs and from every core that implemented it. F is now played as exact Hz.

- Affected, now exact: `chips/3hs816` (HS step `ceil(F·R·2^20/48000)`), `chips/3hs88pwn4` (WT/PCM step `ceil(F·2^32/1500)`), the Rust WASM card `examples/wasm-3ws88pwn4` (round instead of ceil, as that card always rounded), and Furnace's vendored copies (S3HS, 3HS816). Specs: `doc/spec_3HS816.md`, `doc/spec_3HS88PWN4.md`.
- Not affected on purpose: the SGU/S3W2 already steps linearly in F; 34SI22's register *is* a divider (`48000/(DIV+1)`), not a Hz value.
- **Legacy comparisons only hold at divisors of 192000.** The float reference still quantizes, so Close/Info scenarios use 480, 250, 200, 320, 240, 120 Hz… (the Exact tier's 750/375 Hz already were). Picking 440 Hz again will make them fail for a legitimate pitch difference, not a bug. Furnace's `render_check.py --reference` uses song tuning 480 for the same reason.
- Unit tests pin the new behaviour where the old rule was far off: 11000 Hz (old 11294) on HS, 7000 Hz (old 7111) on WT, A7 = 3520 Hz (old 3555.6) in Furnace. Furnace's rendered pitch reads about 0.06 % low through the zero-crossing estimator on both old and new builds; check relative tolerances, not ±0.2 Hz.
- The ceil rounding of steps stays: it is about reaching exact table boundaries, not about pitch quantization.
