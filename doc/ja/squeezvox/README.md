---
lang: ja
translation_of: ../../squeezvox/README.md
---
# SQV4 ファイルコンバーター

Rust の `sqv4_convert` コマンドは、非圧縮16ビット PCM WAV ファイルを SQV4 としてエンコードし、エンコード済みファイルをデコードして往復変換した WAV プレビューを書き出します。

```text
cargo run --manifest-path ../../src32/Cargo.toml --bin sqv4_convert -- input.wav output.sqv4 [H|L|L+] [VOLUME]
```

入力 WAV は、モノラルまたはステレオ、非圧縮符号付き16ビット PCM、かつサンプルレートが0以外でなければなりません。コンバーターはサンプルレートを保持し、リサンプリングしません。出力拡張子は `.sqv` または `.sqv4` でなければなりません。新たにエンコードされるファイルは SQV4 バージョン2を使用し、デコーダーはバージョン1との互換性を維持します。バリアントの既定値は `H`、音量の既定値は `255` で、音量は `0..=255` でなければなりません。プレビューは SQV4 ファイルと同じ場所に `<output-stem>.preview.wav` として書き出されます。

例:

```text
cargo run --manifest-path ../../src32/Cargo.toml --bin sqv4_convert -- voice.wav voice.sqv4 L+ 192
```
