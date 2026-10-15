---
lang: ja
translation_of: README.md
---
# 仮想サイクルタイマー拡張カード

このカードは、リロード周期、開始／停止制御、クリアまで保持される IRQ ステータス、および読み取り専用のパッケージラベルを公開します。タイマーの期限には壁時計時刻ではなく、CPT32 の仮想サイクルを使用します。

| オフセット | 読み取り | 書き込み |
|---|---|---|
| `0x00` | 周期の下位バイト | 周期の下位バイト |
| `0x01` | 周期の上位バイト | 周期の上位バイト |
| `0x02` | 有効時は `1` | ビット 0 で開始（`1`）または停止（`0`） |
| `0x03` | ビット 0 が保留状態を示す | ビット 0 を書き込むと保留状態をクリアし、IRQ を解除 |
| `0x10+` | アセット ID 0 のバイト。範囲外の読み取り値はゼロ | 無視 |

周期にゼロを書き込むと、最小値として1サイクルが選択されます。開始すると、各期限で保留フラグを設定し、カード IRQ をアサートして、次の期限をスケジュールします。ゲストはステータスに `1` を書き込んでレベルをクリアします。同梱の `assets/label.txt` は、パッケージアセットの遅延読み取りを示します。

このディレクトリでビルドおよびパッケージ化します。

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
python3 ../../sdk/wasm-expansion-card/package.py \
  --asset assets/label.txt=assets/label.txt \
  target/wasm32-unknown-unknown/release/cpt32_wasm_timer_card.wasm \
  target/timer.cpt32card
```

`config.yaml` からの相対パスを使用してスロット0を設定します。

```yaml
expansion_card:
  - slot: 0
    package: "plugins/timer.cpt32card"
```

SDK とホストの契約全体については、[`doc/WASM_expansion_card_development.md`](../../doc/WASM_expansion_card_development.md) を参照してください。
