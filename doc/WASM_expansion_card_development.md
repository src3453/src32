# WASM expansion-card 開発ガイド

CPT32 の拡張カードを Rust の `no_std` WASM module として作り、`.cpt32card` package にして実行する手順です。ABI の完全な規則は [WASM Expansion Card ABI v1](spec_WASM_expansion_card.md) を参照してください。

## SDK とプロジェクト

リポジトリには次を用意しています。

- [`sdk/wasm-expansion-card`](../sdk/wasm-expansion-card/): ABI export macro、host API、`.cpt32card` package builder。
- [`templates/wasm-expansion-card`](../templates/wasm-expansion-card/): 256 byte register bank を持つ最小カード。
- [`examples/wasm-expansion-timer`](../examples/wasm-expansion-timer/): 仮想cycle timer、IRQ、lazy package asset を使う実例。
- [`examples/wasm-psg`](../examples/wasm-psg/): 外部crate `psg` をCargo依存として使うAY/YM音源カード。MMIO、音声、受動debug snapshotを実装。

SDK は Rust 2024、`wasm32-unknown-unknown` target、`no_std` を前提にします。プロジェクトは `cdylib` とし、WASI runtime を使わずホストとのやり取りを ABI のみで行います。

## 最小カードを build する

Rust toolchain に WASM target を追加し、starter project を build します。

```sh
rustup target add wasm32-unknown-unknown
cd templates/wasm-expansion-card
cargo build --release --target wasm32-unknown-unknown
```

生成物を package にします。

```sh
python3 ../../sdk/wasm-expansion-card/package.py \
  target/wasm32-unknown-unknown/release/my_cpt32_card.wasm \
  target/my-cpt32-card.cpt32card
```

builder は ABI v1 manifest と module を含む ZIP を作ります。追加 asset は `--asset PACKAGE_PATH=SOURCE_FILE` で指定し、指定順に ID `0` から割り当てます。package 内 path は root 相対の `/` 区切りです。

```sh
python3 ../../sdk/wasm-expansion-card/package.py \
  --asset assets/label.txt=assets/label.txt \
  target/wasm32-unknown-unknown/release/my_cpt32_card.wasm \
  target/my-cpt32-card.cpt32card
```

asset は合計 64 MiB 以下でなければなりません。builder は重複・予約 path、欠落ファイル、asset 上限超過を拒否します。package の module path は既定で `wasm/card.wasm` です。

## カード実装

`cpt32_wasm_expansion_card::Card` を実装し、`export_card!(YourType)` を module に一度だけ置きます。カード状態は `Default` で初期化され、ABI `cpt32_abi_version` 呼び出し時に生成されます。各 entry point は順番に呼ばれ、共有 memory と並行呼び出しは ABI で禁止されています。

```rust
use cpt32_wasm_expansion_card::Card;

#[derive(Default)]
struct MyCard { value: u8 }

impl Card for MyCard {
    fn reset(&mut self) { self.value = 0; }
    fn mmio_read(&mut self, offset: u32) -> u8 {
        if offset == 0 { self.value } else { 0 }
    }
    fn mmio_write(&mut self, offset: u32, value: u8) {
        if offset == 0 { self.value = value; }
    }
    fn tick(&mut self) {}
}

cpt32_wasm_expansion_card::export_card!(MyCard);
```

WASM binary には `cpt32_abi_version`, `cpt32_reset`, `cpt32_mmio_read`, `cpt32_mmio_write`, `cpt32_tick` の5 export が必要です。macro が型と名前を合わせて生成します。crate は `#![no_std]` とし、最終 binary に panic handler を用意してください。starter/example の panic handler は回復不能 panic 時に停止し、host fuel limit により無限 loop は上限で trap します。

外部crateが `std` を使う場合も `wasm32-unknown-unknown` でbuildでき、最終moduleがABIのimport制限を満たせば利用できます。adapterの `#![no_std]` は依存先までno_stdにするものではありません。`std` がpanic実装を供給する場合、独自panic handlerは重複定義になるため追加しません。WASI/OS APIが利用可能になるわけではなく、buildに加えて実hostでpackageをロード・実行して検証してください。[PSG example](../examples/wasm-psg/README.md)に依存と検証手順があります。

`Card::mmio_read/write` は slot 相対 byte offset を受け取ります。slot window は 32 MiB ですが、デバイスが使わない offset の read/write はカード側で定義してください。read は `u8` なので SDK 経由では ABI 範囲外値を返しません。

デバッガのメモリモニタ用に、副作用なしで観測できる範囲があれば `Card::debug_read(&self, offset) -> Option<u8>` を実装してください。`None` を返したoffsetはOpen Bus表示になります。これはCPU/DMA用の `mmio_read` とは独立したsnapshot経路であり、カード状態やhost状態を変更してはいけません。未実装のカードは従来どおりロードでき、Debug Readは未対応になります。

## Host API

### 音声を出力するカード

SDKの `Card::render_audio(&mut self, sample_rate: u32, output: &mut [[i16; 2]])` をoverrideする。
`output` は1〜256 stereo framesで、呼び出し前にゼロに初期化される。既定実装は無音。
`export_card!` は必須5 exportに加えて任意の `cpt32_audio_render` exportとbufferを提供する。
bufferの管理やpointer処理をカードが直接実装する必要はない。サンプルレートはhostから受け取り、
チップ内部の固定rateと異なる場合はカードでresampleする。renderの途中でMMIO/tickは呼ばれない。
フレーム内の書き込みタイミング精度は [ABI音声契約](spec_WASM_expansion_card.md#32-任意の音声出力export) を参照。

実例は [`examples/wasm-3ws88pwn4`](../examples/wasm-3ws88pwn4/)。固定小数点位相の8ch×8op HSと4ch PCM/WT/noise、EQ/compressorを48 kHzで生成する。

SDK の public function は ABI v1 の `cpt32_host` import に対応します。

- `current_cycles() -> u64`: 単調増加する仮想 cycle 値。
- `schedule_tick(delay: u64) -> Result<(), DelayOutOfRange>`: 呼び出し時点からの相対 delay。`0` は既存予約の取り消し。1件だけ予約でき、再予約で置換します。`i64::MAX` を超える値は host を呼ぶ前に error になります。
- `irq_set(asserted: bool)`: IRQ level を assert/deassert。
- `asset_count() -> u32`, `asset_size(id) -> u32`, `asset_read_byte(id, offset) -> u8`: manifest 宣言済み asset への読み取り。asset ID/offset が範囲外なら host trap になります。asset 内容は初回 read 時に展開されます。

各 slot の IRQ は IRQC source `5 + slot` につながります。IRQ level の deassert は IRQC pending bit を消去しません。guest が pending を処理する場合、IRQC の仕様に従って別途 acknowledge してください。

### Timer example の MMIO

[`examples/wasm-expansion-timer`](../examples/wasm-expansion-timer/) は次のレジスタを実装しています。

| Slot offset | Read | Write |
|---|---|---|
| `0x00` | period low | period low |
| `0x01` | period high | period high |
| `0x02` | enable state | bit 0: start / stop |
| `0x03` | pending bit | bit 0 を書くと pending clear と IRQ deassert |
| `0x10+` | manifest asset ID 0 の bytes | ignored |

周期は仮想 cycle 単位で、0 は最小 1 cycle として扱います。各 tick は status を立てて IRQ を assert し、次回 tick を予約します。

## Emulator に装着

`.cpt32card` を config file から参照します。既定の起動経路は current directory の `config.yaml` を読み込み、package 相対 path はその file の directory を基準にします。

```yaml
expansion_card:
  - slot: 0
    package: "plugins/timer.cpt32card"
```

カードが利用できる address は `0xA0000000 + slot * 0x02000000` から始まる 32 MiB window です。最大8 slot、hotplug なしです。カードへの byte access は MMIO export を通ります。既存 DMAC からも slot 内に収まる beat を endpoint として使えます。必要なら通常のゲスト program を起動し、上表の register を操作してください。

```sh
cargo run --release --bin cpt32 -- path/to/program.bin
```

package、manifest、module、import/export に問題があれば起動時に拒否されます。guest trap、fuel 枯渇、host API の規約違反はカードを fault 状態にし、以後の guest call は行いません。fault 後の read は `0`、write は無視され、CPU access は Bus Error になります。

## 実行制限と package の注意

- ABI v1 module は `cpt32_host` の許可 import のみ使えます。WASI、任意 host function、filesystem access はありません。
- WASM memory はカードあたり最大 64 MiB、各 guest invocation は最大 10,000,000 fuel です。module start function にも fuel が適用されます。
- `.cpt32card` は root に `manifest.yaml` を持つ ZIP です。manifest と module は起動時に読み込まれ、asset は要求時に読み込まれます。
- asset path や package 内容をカードから任意指定することはできません。必要な asset は manifest に登録して package builder に渡してください。
- カード reset は tick deadline を取り消し、IRQ level を low にしてから `Card::reset` を呼びます。仮想 cycle counter 自体は巻き戻りません。

## SDK の確認

リポジトリ root から package builder の回帰テストと両 project の WASM build を実行できます。

```sh
python3 -m unittest discover -s sdk/wasm-expansion-card -p 'test_*.py' -v
cargo build --manifest-path templates/wasm-expansion-card/Cargo.toml --release --target wasm32-unknown-unknown
cargo build --manifest-path examples/wasm-expansion-timer/Cargo.toml --release --target wasm32-unknown-unknown
```
