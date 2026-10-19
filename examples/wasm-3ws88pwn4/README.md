# 3WS88PWN4 sound card

仕様は [`doc/spec_3WS88PWN4.md`](../../doc/spec_3WS88PWN4.md)。Furnaceの3HS88PWN4をもとにした独立したRust音源で、位相とPCM/WT cursorは32.32固定小数点。ADSR、13 synthesis modes、16 HS waves、4 PCM/WT/noise/FIFO channels、filter、EQ、compressorを備える。

カードID `3WS88PWN4` はslot相対 `0x4003F0..0x4003F8` のread-only ASCII bytesで公開する。SGUB v3 C driverはslot 0のこのIDを確認し、不一致時だけエラーを表示して終了する。

## Build / package

repository rootで実行:

```powershell
rustup target add wasm32-unknown-unknown
cargo build --manifest-path examples/wasm-3ws88pwn4/Cargo.toml --release --target wasm32-unknown-unknown
python sdk/wasm-expansion-card/package.py examples/wasm-3ws88pwn4/target/wasm32-unknown-unknown/release/cpt32_3ws88pwn4.wasm examples/wasm-3ws88pwn4/target/3ws88pwn4.cpt32card
```

`config.yaml` の既存設定に追加:

```yaml
expansion_card:
  - slot: 0
    package: "examples/wasm-3ws88pwn4/target/3ws88pwn4.cpt32card"
```

slot 0のHS ch1は `0xA0400000`。最小の発音例はF=750 (`+0=2,+1=238`)、OP1 volume (`+0x10=255`)、OP1 sustain (`+0x22=255`)、中立feedback (`+0x1F=128`)、Gate (`+0x1E=1`)。Feedback Destination Registerは `0xA04003C0` から8ch分で、値0〜7がOperator 1〜8を選択する。Feedbackは選択OP自身の直近2サンプルを位相へ戻す。その他は0でよい。同じGateに1を再書き込みすると位相、ADSR、feedback履歴をretriggerする。0を書くとreleaseする。PCM/WT ch1のsyncは `0xA0400208` への任意write。

## Validation

```powershell
cargo test --manifest-path examples/wasm-3ws88pwn4/Cargo.toml
cargo test --test wasm_expansion
$env:CPT32_SOUND_CARD_PACKAGE=(Resolve-Path examples/wasm-3ws88pwn4/target/3ws88pwn4.cpt32card).Path
cargo test --test wasm_expansion built_sound_card -- --ignored
```

`tests/legacy.pcm` は元C++実装から出力した49×512 stereo frames、16-bit LE。
`tests/reference.cpp` は再生成用harnessで、参照元を変更しない。Visual Studio developer promptで、次のように `REFERENCE_SOURCE` に元sound.cppの絶対pathを与える (パスに空白がある場合はcompilerの引用規則に従う)。

```bat
cl /std:c++17 /EHsc /O2 /DREFERENCE_SOURCE=\"C:/path/to/furnace/src/engine/platform/sound/s3hs88pwn4/sound.cpp\" examples\wasm-3ws88pwn4\tests\reference.cpp /Fe:reference.exe
reference.exe examples\wasm-3ws88pwn4\tests\legacy.pcm
```

比較用pitchは固定小数点で正確に表せる値を選んでいる。一般のpitch、feedbackについて旧float実装とのbit一致を保証するfixtureではない。新実装の長時間位相carryは整数モデルとの別テストで確認する。
