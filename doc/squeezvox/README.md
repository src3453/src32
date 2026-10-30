# SQV4 file converter

The Rust `sqv4_convert` command encodes uncompressed 16-bit PCM WAV files as SQV4 and writes a round-trip WAV preview decoded from the encoded file.

```text
cargo run --manifest-path ../../src32/Cargo.toml --bin sqv4_convert -- input.wav output.sqv4 [H|L|L+] [VOLUME]
```

Input WAV must be mono or stereo, uncompressed signed 16-bit PCM, with a nonzero sample rate. The converter preserves the sample rate and does not resample. Output extension must be `.sqv` or `.sqv4`; newly encoded files use SQV4 version 3, whose fragments are normalized to their peak and decode independently; the preview decoder also reads versions 1 and 2. Variant defaults to `H`; volume is a pre-gain (`x * volume / 255`) applied before normalization, defaults to `255`, and must be in `0..=255`. SGU v3 plays only version 3 files; SGU v1/v2/v2.1 play only versions 1 and 2. The preview is written beside the SQV4 file as `<output-stem>.preview.wav`.

Example:

```text
cargo run --manifest-path ../../src32/Cargo.toml --bin sqv4_convert -- voice.wav voice.sqv4 L+ 192
```