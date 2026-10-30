---
lang: ja
translation_of: ../../squeezvox/README.md
---
# SQV4 ファイルコンバーター

Rust の `sqv4_convert` コマンドは、非圧縮16ビット PCM WAV ファイルを SQV4 としてエンコードし、エンコード済みファイルをデコードして往復変換した WAV プレビューを書き出します。

```text
cargo run --manifest-path ../../src32/Cargo.toml --bin sqv4_convert -- input.wav output.sqv4 [H|L|L+] [VOLUME]
```

入力 WAV は、モノラルまたはステレオ、非圧縮符号付き16ビット PCM、かつサンプルレートが0以外でなければなりません。コンバーターはサンプルレートを保持し、リサンプリングしません。出力拡張子は `.sqv` または `.sqv4` でなければなりません。新たにエンコードされるファイルは SQV4 バージョン3を使用します。バージョン3のフラグメントはピークに合わせて正規化され、単独で復号できます。プレビューのデコーダーはバージョン1・2も読めます。バリアントの既定値は `H` です。音量は正規化の前に掛けるプリゲイン（`x * volume / 255`）で、既定値は `255`、範囲は `0..=255` でなければなりません。SGU v3 はバージョン3のファイルだけを、SGU v1／v2／v2.1 はバージョン1・2だけを再生します。プレビューは SQV4 ファイルと同じ場所に `<output-stem>.preview.wav` として書き出されます。

例:

```text
cargo run --manifest-path ../../src32/Cargo.toml --bin sqv4_convert -- voice.wav voice.sqv4 L+ 192
```
