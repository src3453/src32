# SqueezVOX 4 (SQV4) versions 1 and 2 specification

This document defines the normative version 1 and version 2 wire formats and predictor rules for SQV4H, SQV4L, and SQV4L+. Version 2 preserves version 1 decoding except for the L+ codebook, which is versioned because changing its reconstruction levels changes decoded PCM. New encoders MUST write version 2; decoders MUST support versions 1 and 2. Multi-byte integers are unsigned little-endian unless stated otherwise. PCM input and output are signed 16-bit little-endian interleaved samples. A decoder MUST reject malformed or non-canonical file lengths; encoders MUST emit exactly the format defined here.

## File format

The fixed header is 16 bytes:

| Offset | Size | Meaning |
| --- | ---: | --- |
| 0 | 4 | ASCII `SQV4` |
| 4 | 1 | Version: `1` or `2`; encoders MUST write `2` |
| 5 | 1 | Variant: `0` = H, `1` = L, `2` = L+ |
| 6 | 1 | Channel count: `1` mono or `2` stereo |
| 7 | 1 | Reserved, MUST be zero |
| 8 | 4 | Sample rate in Hz, MUST be nonzero |
| 12 | 4 | Samples per channel |

An empty stream has a zero sample count and consists of the header alone. The declared sample rate is metadata; decoding does not resample.

The payload consists of time-ordered fragments. For each fragment, channels appear in ascending channel order. Each channel fragment is one unsigned volume byte followed by 64 packed codes. H uses 4-bit codes (32 payload bytes); L and L+ use 3-bit codes (24 payload bytes). Codes are packed least-significant-bit first: code bit 0 is the first bit in the stream, and samples are packed in ascending sample index. Unused code slots in the final fragment MUST be zero. A decoder emits only the declared per-channel sample count; padded slots do not contribute output.

Each fragment volume is a linear gain in `[0,255]`. Decoding applies signed integer division toward zero: `output = reconstructed_predictor * volume / 255`. An encoder's `volume` argument is written unchanged to every channel fragment.

For `n` samples per channel, `f = ceil(n/64)` fragments per channel are present, including zero fragments when `n=0`. The exact file size is `16 + f * channel_count * (1 + payload_bytes_per_fragment)`, where payload size is 32 for H and 24 for L/L+. Arithmetic MUST be checked for overflow. A decoder MUST reject a short header, bad magic/version/variant/channel count/reserved byte, zero rate, truncated payload, or any trailing byte.

Each channel has an independent predictor and step index, initialized once at file start to predictor `0`, index `0`, then retained across all its fragments. Fragments carry no predictor state; random frame seek is not guaranteed. Stereo channel state is independent.

## Shared ADPCM state

After determining a code's signed difference `delta`, update `predictor = clamp(predictor + delta, -32768, 32767)`, then update `index = clamp(index + adjustment, 0, 88)`. The reconstructed sample for a code is the updated predictor. Encoder code selection evaluates this post-clamp reconstructed sample. The initial state and all state transitions apply identically to encoding and decoding.

### SQV4H

H is IMA ADPCM. Code bit 3 is sign (`1` means subtract); bits 0–2 are magnitude. Its 89-entry step table is:

`[7,8,9,10,11,12,13,14,16,17,19,21,23,25,28,31,34,37,41,45,50,55,60,66,73,80,88,97,107,118,130,143,157,173,190,209,230,253,279,307,337,371,408,449,494,544,598,658,724,796,876,963,1060,1166,1282,1411,1552,1707,1878,2066,2272,2499,2749,3024,3327,3660,4026,4428,4871,5358,5894,6484,7132,7845,8630,9493,10442,11487,12635,13899,15289,16818,18500,20350,22385,24623,27086,29794,32767]`

For magnitude `m = code & 7` and current step `s`, `diff = (s >> 3) + ((m & 1) != 0 ? s >> 2 : 0) + ((m & 2) != 0 ? s >> 1 : 0) + ((m & 4) != 0 ? s : 0)`. The index adjustment indexed by the full code is `[-1,-1,-1,-1,2,4,6,8,-1,-1,-1,-1,2,4,6,8]`.

The H encoder compares the absolute input-predictor difference with `s`, `s/2`, and `s/4` in that order; each threshold met sets magnitude bit 2, 1, or 0 respectively and subtracts that threshold from the remaining difference. The sign bit is set iff the input is below the current predictor.

### SQV4L

L uses 3-bit codes: bit 2 is sign (`1` means subtract), bits 0–1 select magnitude. For magnitude `m` in `[0,3]`, the positive reconstruction level is `[(1*s+4)/8, (3*s+4)/8, (5*s+4)/8, (7*s+4)/8]`, using integer division. The sign negates this level. The magnitude-index adjustment is `[-1,2,4,6]`.

### SQV4L+

L+ uses the same 3-bit sign, magnitude, predictor update, and index adjustment as L. Its positive difference is `(level*s+4)/8`; code bit 2 negates the difference.

Version 1 uses codebook `[10,11,12,14]`. Version 2 uses codebook `[1,2,3,14]`. A decoder MUST select the codebook by the file version; an encoder MUST use the version 2 codebook and write version `2`. This preserves playback of existing version 1 L+ files while correcting version 2 audio quality.

Both codebooks are selected from all integer tuples `(a,b,c,d)` with `1 <= a < b < c < d <= 15`, excluding arithmetic progressions. For each tuple, encode then decode each fixture in sequence with predictor/index reset per fixture and volume 255; choose the tuple minimizing total squared error, ties lexicographically.

The version 1 benchmark uses five 1024-sample mono fixtures: silence (all zero); impulse (sample 0 = 32767, sample 511 = -32768, all others zero); sawtooth (`-32768 + (i mod 256)*256`); ramp (`-32768 + floor(i*65535/1023)`); square (`i mod 128 < 64 ? 24576 : -24576`). It selects `[10,11,12,14]` with total squared error `252189726976` over 5120 samples.

The version 2 benchmark uses those five fixtures plus an 8192-sample tone fixture formed by repeating this 128-sample signed PCM table 64 times:

`[0,8926,15150,17259,15693,12291,9111,7209,6123,4321,355,-5996,-13304,-19056,-20923,-17999,-11314,-3305,3432,7423,8889,9316,10311,12445,14782,15391,12542,5883,-3121,-11761,-17368,-18600,-16000,-11529,-7368,-4690,-3121,-1188,2542,8320,14782,19516,20311,16387,8889,352,-6568,-10376,-11314,-10928,-10923,-11985,-13304,-13067,-9645,-2750,6123,14280,19111,19362,15693,10188,5150,1855,0,-1855,-5150,-10188,-15693,-19362,-19111,-14280,-6123,2750,9645,13067,13304,11985,10923,10928,11314,10376,6568,-352,-8889,-16387,-20311,-19516,-14782,-8320,-2542,1188,3121,4690,7368,11529,16000,18600,17368,11761,3121,-5883,-12542,-15391,-14782,-12445,-10311,-9316,-8889,-7423,-3432,3305,11314,17999,20923,19056,13304,5996,-355,-4321,-6123,-7209,-9111,-12291,-15693,-17259,-15150,-8926]`

The version 2 winner is `[1,2,3,14]`, with total squared error `279448221728` over 13312 samples. Codebook levels MUST remain fixed during encoding and decoding; decoders MUST NOT optimize them at runtime.

## Encoder selection

For L and L+, at each input sample the encoder evaluates every legal magnitude with both signs. It applies the candidate difference to the current predictor with 16-bit clamping, then selects the code minimizing absolute error to the input sample. Ties select the numerically smallest code. It emits the code and updates predictor and index using the normal state rules before processing the next sample. H uses the threshold quantizer specified above. Encoders write zero codes for final-fragment padding without advancing predictor/index for those nonexistent samples.

## Hardware playback container

The S3W2 SQV4 waveform consumes a file placed wholly within PCM RAM. The start register points to the 16-byte file header; the end register is an exclusive byte address covering the complete file. Only mono files are accepted by this hardware playback path. The PCM sample-rate register continues to control playback cadence; the file's sample rate is metadata only. A loop address of zero or equal to start means restart at the file header. Any other loop address MUST point to the first byte of a channel fragment (the volume byte) after the header; it MUST NOT be interpreted as an independent seek state. Looping reconstructs from initial predictor/index state through the selected fragment. Invalid header, non-mono data, out-of-RAM bounds, incomplete file, or invalid loop boundary makes the channel inactive without reading out of bounds and produces silence.