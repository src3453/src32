# SQV4 file converter

The Rust `sqv4_convert` command encodes uncompressed 16-bit PCM WAV files as SQV4 and writes a round-trip WAV preview decoded from the encoded file.

```text
cargo run --manifest-path ../../src32/Cargo.toml --bin sqv4_convert -- input.wav output.sqv4 [H|L|L+] [VOLUME]
```

Input WAV must be mono or stereo, uncompressed signed 16-bit PCM, with a nonzero sample rate. The converter preserves the sample rate and does not resample. Output extension must be `.sqv` or `.sqv4`; newly encoded files use SQV4 version 2, and the decoder remains compatible with version 1. Variant defaults to `H`; volume defaults to `255` and must be in `0..=255`. The preview is written beside the SQV4 file as `<output-stem>.preview.wav`.

Example:

```text
cargo run --manifest-path ../../src32/Cargo.toml --bin sqv4_convert -- voice.wav voice.sqv4 L+ 192
```