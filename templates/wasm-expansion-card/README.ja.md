---
lang: ja
translation_of: README.md
---
# CPT32 WASM 拡張カードのスターター

この `no_std` Rust プロジェクトは、オフセット `0x00..0xFF` にある256個のバイト幅レジスターを備えた ABI v1 カードをエクスポートします。それ以外のオフセットの読み取り値はゼロで、書き込みは無視されます。`MyCard` をデバイス状態に置き換え、`Card` を実装してから、このディレクトリでビルドおよびパッケージ化します。

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
python3 ../../sdk/wasm-expansion-card/package.py \
  target/wasm32-unknown-unknown/release/my_cpt32_card.wasm \
  target/my-cpt32-card.cpt32card
```

`Cargo.toml` 内の SDK パスは、このリポジトリからの相対パスです。プロジェクトを別の場所に移動する場合は、`sdk/wasm-expansion-card` も同梱し、そのパスとパッケージ化スクリプトのパスを更新してください。ABI の使用方法、アセット、タイミング、IRQ、パッケージ化、エミュレーターの設定については、[`doc/WASM_expansion_card_development.md`](../../doc/WASM_expansion_card_development.md) を参照してください。
