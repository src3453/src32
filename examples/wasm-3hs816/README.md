# 3HS816 sound card

仕様は [`doc/spec_3HS816.md`](../../doc/spec_3HS816.md)（「CPT32 拡張カード」の節）。`chips/3hs816/` の整数 C++ モデルを Rust に移植した WASM 拡張カードで、出力は C++ モデルとビット一致する。slot 相対 `0x000..0x4FF` がチップのレジスタそのもので、ID `3HS816` + `01 00` は `0x4F8..0x4FF`（読み出し専用）。音声は 48 kHz で生成し、ホストのレートへ零次ホールドで変換する。IRQ・tick・asset は使わない。

SGUB v4（[`doc/spec_SGUB_v4.md`](../../doc/spec_SGUB_v4.md)）の Furnace exporter は、曲の 3HS816 をこのカードへの書き込みとして出力する。

## Build / package

repository root で実行:

```powershell
rustup target add wasm32-unknown-unknown
cargo build --manifest-path examples/wasm-3hs816/Cargo.toml --release --target wasm32-unknown-unknown
python sdk/wasm-expansion-card/package.py examples/wasm-3hs816/target/wasm32-unknown-unknown/release/cpt32_3hs816.wasm examples/wasm-3hs816/target/3hs816.cpt32card
```

`config.yaml` に追加（slot は SGUB のカード要件表に合わせる）:

```yaml
expansion_card:
  - slot: 0
    package: "examples/wasm-3hs816/target/3hs816.cpt32card"
```

## Validation

```powershell
cargo test --manifest-path examples/wasm-3hs816/Cargo.toml
```

- `matches_cpp_reference_bit_exactly`: `tests/script.txt`（16 チャンネル、全アルゴリズム、ユーザー波形、フィードバック、パン・ミュート、リトリガーとリリース）を再生し、C++ モデルの出力 `tests/reference.pcm`（16-bit LE stereo）と全サンプルを比べる。
- `tables_match_cpp_header`: `src/tables.rs` が `chips/3hs816/src/generated_tables.hpp` と一致すること。表を変えたら `python examples/wasm-3hs816/tools/gen_tables.py` で作り直す。

C++ モデルを変えたら Rust 側も同じ変更を入れ、fixture を作り直す。`tests/gen_script.py` が script を、`tests/reference.cpp` が fixture を作る（`chips/3hs816/src/*.cpp` と一緒にビルドし、`-I chips/3hs816/include -I chips/3hs816/src`）:

```powershell
python examples/wasm-3hs816/tests/gen_script.py
reference.exe examples/wasm-3hs816/tests/script.txt examples/wasm-3hs816/tests/reference.pcm
```
