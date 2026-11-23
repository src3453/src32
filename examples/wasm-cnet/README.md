# CNET network card

CPT32 用ネットワークの WASM 拡張カードです。データリンク層（ノード番号の取得）とネットワーク層（データグラム、ソケット、CRC-16、エコー応答）はこのカードの中で動きます。エミュレータ側は、設定されたセグメントへ生のリンクフレームを運ぶだけです。仕様は [`doc/spec_NET.md`](../../doc/spec_NET.md) を参照してください。

## ビルドとパッケージ

```sh
cargo build --manifest-path examples/wasm-cnet/Cargo.toml --release --target wasm32-unknown-unknown
python sdk/wasm-expansion-card/package.py --capability network --capability gui \
  examples/wasm-cnet/target/wasm32-unknown-unknown/release/cpt32_wasm_cnet_card.wasm \
  plugins/cnet.cpt32card
```

## 装着

```yaml
network_segment:
  - name: lan
    network: 0x0001
    udp:
      bind: "0.0.0.0:32032"
      multicast: "239.255.32.32:32032"

expansion_card:
  - slot: 2
    package: "plugins/cnet.cpt32card"
    network:
      segment: lan
      # node: 0x0012   # 省略するとノード番号を自動取得
```

`network` を書かないとカードはセグメントにつながらず、外部との通信は一切起きません。同じ PC で複数のエミュレータを動かす場合も、同じマルチキャストグループを使えばつながります。マルチキャストが届かない環境では、`udp.peers` に相手の `アドレス:ポート` を列挙します。

## パネル

`gui` capability を宣言しているので、エミュレータの Expansion Cards → Overview の表に Panel ボタンが出ます。アドレス、送受信のバイト数と速度、開いているソケットを表示し、リンクの有効化、ノード番号のヒント、アドレスの取り直し、模擬パケットロスを操作できます（[`doc/spec_NET.md`](../../doc/spec_NET.md) §8.4）。

## ゲストからの使い方（概略）

1. `CONTROL`（`0x08`）に `0x0001` を書いてリンクを有効にし、`STATUS`（`0x06`）の ADDR_VALID（bit2）か IRQ の ADDR イベントを待ちます。
2. ハンドル `h` の `SOCKET`（`0x100 + h*0x20`）にソケット番号を書き（0 なら動的割当）、`CMD` に `1`（OPEN）を書きます。
3. 送信は TX バッファ（`0x10000 + h*0x400`）にペイロードを書き、`TX_NET`/`TX_NODE`/`TX_SOCKET`/`TX_PROTO`/`TX_LEN` を設定して `CMD` に `3`（SEND）を書きます。結果は `RESULT` に入ります。
4. 受信は `RX_COUNT` か IRQ の RX イベントで知り、`RX_*` と RX バッファ（`0x20000 + h*0x400`）を読んでから `CMD` に `4`（RX_POP）を書きます。

テスト: `cargo test --test cnet`（先にこのカードをビルドしておきます）。
