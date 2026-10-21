# PSG WASM拡張カード（外部crateの利用例）

[thedjinn/psg-rs](https://github.com/thedjinn/psg-rs) の公開crate
[`psg` 1.0.1](https://docs.rs/psg/1.0.1/psg/) をCargo依存として使う音源カード。
ソースのコピーやホスト側音源の追加は不要。SDKの `Card` と `export_card!` で
MMIOと標準音声ABIに接続する。上流はMIT license。

## Build / package

リポジトリrootで実行する（初回はcrate取得のネットワーク接続が必要）。

```sh
rustup target add wasm32-unknown-unknown
cargo build --manifest-path examples/wasm-psg/Cargo.toml --release --target wasm32-unknown-unknown
python sdk/wasm-expansion-card/package.py examples/wasm-psg/target/wasm32-unknown-unknown/release/cpt32_psg.wasm examples/wasm-psg/target/psg.cpt32card
```

rootの `config.yaml` の既存設定に追加する。

```yaml
expansion_card:
  - slot: 0
    package: "examples/wasm-psg/target/psg.cpt32card"
```

ゲストから下記レジスタを設定するとGUIの共通音声ミキサーへ出力される。
testbenchは自動で音声renderしない。headless検証には下記integration testを使う。

## カード仕様

slot baseは `0xA0000000 + slot * 0x02000000`。すべてbyteアクセス。

| Slot offset | Read | Write |
|---|---|---|
| `0x00..0x0F` | PSG R0..R15の書込ラッチ | PSGレジスタ直接書込 |
| `0x10` | 選択レジスタ番号 | address port、下位4 bit |
| `0x11` | 選択レジスタのラッチ | data port |
| `0x12` | model | bit 0: 0=YM2149（既定）、1=AY-3-8910 |
| `0xF0..0xF7` | ASCII `PSG-RS01` | 無視 |
| その他 | 0（debug snapshotは利用不可） | 無視 |

R0/1、R2/3、R4/5は各toneの12-bit period、R6は5-bit noise period、
R7はmixer（bit 0..2=tone disable、bit 3..5=noise disable）、
R8..10は振幅（bit 0..3）とenvelope enable（bit 4）、
R11/12は16-bit envelope period、R13は4-bit envelope shape。
R14/15はラッチのみで外部GPIOは接続しない。未使用bitはmaskして読み戻す。
period=0は上流に合わせ実効値1。tone/envelope periodは両byteのラッチから合成するので
書込順に依存しない。R13は同じ値の再書込でもenvelopeを再始動する。

resetはYM、R7=`0x3F`、その他ラッチ=0、選択番号=0、無音に戻し、
位相・filter・resamplerも初期化する。model切替は位相を保持する。
全定義レジスタのdebug snapshotは副作用なし。IRQ/tickは使わない。

クロックは1,789,772.5 Hz、内部生成は48 kHz固定。上流の既定pan（全channel中央）で
monoをstereo両側へ出力し、`f64 * 16384` を飽和i16へ変換する。
host rateには整数位相のzero-order holdで変換し、block間で位相を保持する。
rate変更時もPSG状態を保持し、resampler位相を新rateに換算する。
8 kHz未満は処理量を制限するため状態を進めず無音を返す。
downsampling用の追加low-passはなく、低rateではaliasingが生じる。
MMIOの発音時刻は既存ABIと同じフレーム末レジスタ方式。

## stdを使う外部crateの注意

adapter自身は `#![no_std]` だが、`psg` 1.0.1は `std` を使い、no_std featureはない。
そのため最終WASMには依存先のRust stdもリンクされる。これは依存ツリー全体がno_stdという
意味ではない。WASIは使わず `wasm32-unknown-unknown` を対象とし、hostが実moduleの
import/exportを検査する。panic実装はstdから供給されるため独自panic handlerを追加しない。
releaseのpanicはabort。外部crate一般についても、filesystem/thread/OS APIを使う経路が
そのまま動くわけではなく、最終moduleを実hostへロードして確認する必要がある。

## 検証

```sh
cargo test --manifest-path examples/wasm-psg/Cargo.toml
```

レジスタ/ports/reset、上流との出力一致、period書込順、受動debug read、
block分割の連続性、両modelのenvelope再始動を確認する。
上記build/package後にPowerShellで実moduleのhost integrationを実行する。

```powershell
$env:CPT32_PSG_CARD_PACKAGE = (Resolve-Path examples/wasm-psg/target/psg.cpt32card).Path
cargo test --release --test wasm_psg -- --ignored --nocapture
```

実際のWasmtime経由でABI/import検査、無音、両modelの3音+noise+envelope、
48/44.1/8/96 kHz、fuel制限内のrender、readback、reset、Bus cycle不変を確認する。
物理オーディオ機器の試聴を行うテストではない。

## ゲストCからA4を鳴らす例

既存ゲストの初期化処理から呼ぶ。tone周波数は `clock / (16 * period)` なので
period=254で約440.4 Hzとなる。停止はR8へ0を書く。

```c
static void psg_a4(void) {
    volatile unsigned char *psg = (volatile unsigned char *)0xA0000000u;
    psg[8] = 0;
    psg[0] = 254;
    psg[1] = 0;
    psg[7] = 0x3E; /* A tone on, all noise/B/C tone off */
    psg[8] = 15;
}
```
