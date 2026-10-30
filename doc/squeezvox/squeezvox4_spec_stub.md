# SqueezVOX 4 (SQV4) version 3 specification

This document defines the normative SQV4 version 3 wire format, predictor rules, and encoder rules for the SQV4H, SQV4L, and SQV4L+ variants. Version 3 is a breaking change from versions 1 and 2: every channel fragment is normalized to full scale, carries its own initial step index, and decodes without any earlier fragment, so a decoder can join a stream at any fragment boundary. New encoders MUST write version 3. Versions 1 and 2 are summarized in [Appendix A](#appendix-a-versions-1-and-2) only for decoders that keep playing old files.

Multi-byte integers are unsigned little-endian unless stated otherwise. PCM input and output are signed 16-bit interleaved samples. A decoder MUST reject malformed or non-canonical files as described below; encoders MUST emit exactly the format defined here.

SQV4 version 3 belongs to the SGU v3 generation (`doc/spec_SGU.md` §8): the SGU v3 cores and the 34SI22 (`doc/spec_34SI22.md`) play version 3 only, and SGU v1, v2, and v2.1 play versions 1 and 2 only. The chip revision is unchanged.

## File format

The fixed header is 16 bytes:

| Offset | Size | Meaning |
| --- | ---: | --- |
| 0 | 4 | ASCII `SQV4` |
| 4 | 1 | Version: `3` |
| 5 | 1 | Variant: `0` = H, `1` = L, `2` = L+ |
| 6 | 1 | Channel count: `1` mono or `2` stereo |
| 7 | 1 | Reserved, MUST be zero |
| 8 | 4 | Sample rate in Hz, MUST be nonzero |
| 12 | 4 | Samples per channel |

An empty stream has a zero sample count and consists of the header alone. The declared sample rate is metadata; decoding does not resample.

The payload consists of time-ordered fragments of 63 samples per channel. For each fragment, channels appear in ascending channel order. For `n` samples per channel there are `f = ceil(n/63)` fragments per channel (zero when `n = 0`); the last one holds `n - 63*(f-1)` samples.

## Channel fragment

| Offset | Size | Field |
| --- | ---: | --- |
| 0 | 1 | Volume: linear gain `0..255` |
| 1 | 1 | Index: initial step index `0..88` |
| 2 | 32 (H) / 24 (L, L+) | Codes: 63 packed codes |

H codes are 4 bits and L/L+ codes are 3 bits. Codes are packed least-significant-bit first: bit 0 of code 0 is bit 0 of the first code byte, and samples follow in ascending order. 63 codes occupy 252 bits (H) or 189 bits (L/L+), so the code area is padded to a whole byte with 4 (H) or 3 (L/L+) zero bits. A channel fragment is therefore 34 bytes for H and 26 bytes for L/L+.

The Index field is the ADPCM step index, not a sample value. The predictor (sample value) of every fragment starts at `0` and is not stored, so the 63 codes produce the fragment's 63 output samples; there is no separate initial sample.

The exact file size is `16 + f * channel_count * fragment_size`. Arithmetic MUST be checked for overflow.

In the last fragment, code slots past the last sample MUST be zero; together with the byte padding, every bit after the last used code of a fragment is zero. A decoder emits only the declared per-channel sample count.

## Decoding

Each channel fragment decodes on its own. At the first code of a fragment the channel's state is set to predictor `0` and step index `Index`. Nothing carries over from the previous fragment, and stereo channels are independent.

For each code in order, the decoder computes the variant's signed difference `delta` from the current step `s = STEPS[index]`, then updates `predictor = clamp(predictor + delta, -32768, 32767)` and `index = clamp(index + adjustment, 0, 88)`. The reconstructed value of the code is the updated predictor. The output sample is the reconstructed value scaled by the fragment volume, rounded to nearest with ties away from zero:

`output = sign(p) * ((|p| + 127) / 255)`, where `p = reconstructed * volume` and `/` is integer division.

A volume of `0` decodes to silence.

### SQV4H

H is IMA ADPCM. Code bit 3 is sign (`1` means subtract); bits 0–2 are magnitude. The 89-entry step table `STEPS` is:

`[7,8,9,10,11,12,13,14,16,17,19,21,23,25,28,31,34,37,41,45,50,55,60,66,73,80,88,97,107,118,130,143,157,173,190,209,230,253,279,307,337,371,408,449,494,544,598,658,724,796,876,963,1060,1166,1282,1411,1552,1707,1878,2066,2272,2499,2749,3024,3327,3660,4026,4428,4871,5358,5894,6484,7132,7845,8630,9493,10442,11487,12635,13899,15289,16818,18500,20350,22385,24623,27086,29794,32767]`

For magnitude `m = code & 7`, `diff = (s >> 3) + ((m & 1) != 0 ? s >> 2 : 0) + ((m & 2) != 0 ? s >> 1 : 0) + ((m & 4) != 0 ? s : 0)`. The index adjustment indexed by the full code is `[-1,-1,-1,-1,2,4,6,8,-1,-1,-1,-1,2,4,6,8]`.

### SQV4L

L codes are 3 bits: bit 2 is sign (`1` means subtract), bits 0–1 select magnitude `m`. The positive difference is `(level[m]*s + 4) / 8` with integer division and levels `[1,3,5,7]`. The index adjustment indexed by `m` is `[-1,2,4,6]`.

### SQV4L+

L+ uses the L sign, magnitude, predictor update, and index adjustment with levels `[1,2,3,14]`. The levels are fixed; decoders MUST NOT optimize them at runtime.

The levels were selected from all integer tuples `(a,b,c,d)` with `1 <= a < b < c < d <= 15`, excluding arithmetic progressions, by encoding and decoding six mono fixtures with a continuous predictor (reset per fixture, unit gain) and minimizing total squared error, ties lexicographically. The fixtures are: silence (1024 zero samples); impulse (1024 samples, sample 0 = 32767, sample 511 = -32768, others zero); sawtooth (`-32768 + (i mod 256)*256`, 1024 samples); ramp (`-32768 + floor(i*65535/1023)`, 1024 samples); square (`i mod 128 < 64 ? 24576 : -24576`, 1024 samples); and an 8192-sample tone repeating the 128-sample table `L_PLUS_BENCH_TONE` in `src/devices/sgu/sqv4.rs`. The winner `[1,2,3,14]` has total squared error `279448221728` over 13312 samples (test `l_plus_codebook_benchmark_is_reproducible`).

## Encoding

The encoder processes each channel fragment independently from its own input samples `x[0..k)`, `1 <= k <= 63`. It is deterministic; the 34SI22 recorder and the file encoder produce identical bytes.

1. **Volume.** With `peak = max |x[i]|` (`|-32768| = 32768`), `Volume = min(255, ceil(peak * 255 / 32767))`. A fragment of zeros has volume `0`; the encoder then writes index `0` and all-zero codes.
2. **Normalization.** Each sample is scaled to the fragment's full scale: `n[i] = clamp(round(x[i] * 255 / Volume), -32768, 32767)`, rounding to nearest with ties away from zero. Quiet fragments are thereby quantized with the same relative precision as loud ones, which reduces the quantization error that a fixed gain would leave at low levels.
3. **Code selection** for a given initial index, starting from predictor `0`:
   - H compares the absolute difference between `n[i]` and the predictor with `s`, `s/2`, and `s/4` in that order; each threshold met sets magnitude bit 2, 1, or 0 and is subtracted from the remaining difference. The sign bit is set iff `n[i]` is below the predictor.
   - L and L+ evaluate all eight codes, apply each difference to the predictor with 16-bit clamping, and select the code with the smallest absolute error to `n[i]`; ties select the numerically smallest code.
   - The selected code updates predictor and index by the decoding rules before the next sample.
4. **Index.** The encoder runs step 3 for every initial index `0..88` and keeps the one that minimizes `sum((x[i] - output[i])^2)`, with `output` computed by the decoding rules (volume applied). Ties select the smallest index.

An encoder MAY apply a linear pre-gain to its input before step 1 (the reference `encode_interleaved` and `sqv4_convert` take `volume`: `x * volume / 255`, truncated toward zero); the pre-gain is not stored.

## Validation

A decoder MUST reject: a short header; bad magic; an unsupported version; a variant other than 0–2; a channel count other than 1 or 2; a nonzero reserved byte; a zero sample rate; a file length other than the exact size (truncation or trailing bytes); an index byte above 88; and any nonzero bit after the last used code of a fragment.

A streaming decoder that consumes fragments before the whole file is available (the 34SI22) cannot validate ahead. It MUST still produce the decoding above for valid data; for an index byte above 88 it uses 88.

## Hardware playback container

The S3W2 SQV4 waveform (SGU v3) consumes a version 3 file placed wholly within PCM RAM. The start register points to the 16-byte header; the end register is an exclusive byte address covering the complete file. Only mono files are accepted. The PCM sample-rate register controls playback cadence; the file's sample rate is metadata only. A loop address of zero or equal to start restarts at the first fragment. Any other loop address MUST point to the first byte (volume) of a channel fragment after the header; playback resumes there by decoding that fragment alone. An invalid file (any validation failure above), a file whose version does not match the SGU generation, non-mono data, out-of-RAM bounds, or an invalid loop boundary makes the channel inactive without reading out of bounds and produces silence.

## ImHex inspection pattern

`tools/imhex/sqv4.hexpat` displays the header and variant-sized channel fragments of versions 1–3. Packed ADPCM code bytes remain raw; the pattern does not decode them.

## Appendix A: versions 1 and 2

Versions 1 and 2 share the header above (version byte `1` or `2`). Their channel fragments hold 64 samples: one volume byte followed by 64 codes packed as above (32 bytes for H, 24 for L/L+), with no index byte and no padding inside a full fragment. Each channel's predictor and index start at `0` once at file start and carry across all fragments, so a fragment cannot be decoded alone. The output is `reconstructed * volume / 255` truncated toward zero, and encoders wrote the volume argument unchanged into every fragment (no normalization). The H and L rules are those above; L+ uses levels `[10,11,12,14]` in version 1 and `[1,2,3,14]` in version 2. Unused code slots of the last fragment are zero. A version 1/2 loop address points to a fragment's volume byte and is reached by decoding from the file start.
