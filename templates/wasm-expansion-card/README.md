# CPT32 WASM expansion-card starter

This `no_std` Rust project exports an ABI v1 card with 256 byte-wide registers at offsets `0x00..0xFF`. Other offsets read as zero and ignore writes. Replace `MyCard` with your device state, implement `Card`, then build and package it from this directory:

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
python3 ../../sdk/wasm-expansion-card/package.py \
  target/wasm32-unknown-unknown/release/my_cpt32_card.wasm \
  target/my-cpt32-card.cpt32card
```

The SDK path in `Cargo.toml` is relative to this repository. If you move the project elsewhere, also vendor `sdk/wasm-expansion-card` and update that path and the packaging script path. See [`doc/WASM_expansion_card_development.md`](../../doc/WASM_expansion_card_development.md) for ABI usage, assets, timing, IRQ, packaging, and emulator configuration.
