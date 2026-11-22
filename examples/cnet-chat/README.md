# CNET chat: 2 台のエミュレータで通信するデモ

同じ PC で Foundation のエミュレータを 2 台起動し、CNET ネットワークカード（[`examples/wasm-cnet`](../wasm-cnet/)）で UDP セグメントにつないで、チャットをします。

| | ノード | シェルで入力 |
|---|---|---|
| A | `0001.0010` | `chat 20` |
| B | `0001.0020` | `chat 10` |

`chat` は入力した 1 行を 1 個のデータグラム（ソケット 77、PROTO 0x80）として相手に送り、届いたデータグラムを送信元アドレス付きで入力行の上に表示します。Esc で終了します。相手が同じネットワークにいるときは、アドレスを短縮表記 `.NODE` または `NODE` で書けます（`chat .20` と `chat 20` は `chat 1.20` と同じ）。シェル標準の `cnet ping 20` や `cnet send 20 77 hello` でも通信できます。

## 起動

```sh
python examples/cnet-chat/run_demo.py [--clang PATH]
```

スクリプトは次の順に進みます。

1. Foundation（`foundation/boot/build.py`）、CNET カード（cargo、`wasm32-unknown-unknown`）、`chat.c`（libcnet を import する WAX v3）をビルドします。
2. ディスクイメージに `/bin/chat.wax` を入れます。
3. `target/cnet-demo/A` と `B` に、それぞれのディスクのコピーと `config.yaml` を書きます。2 台は `127.0.0.1` の空きポートで互いを `udp.peers` に指定し合います。
4. `cpt32` を 2 つ起動します。

## ヘッドレス実行（確認用）

```sh
python examples/cnet-chat/run_demo.py --headless
```

`src32_testbench` を 2 つ同時に動かし、両方に会話を入力して、それぞれの画面に相手の行が出ることを確かめます。成功すると両方の画面を表示し、終了コード 0 で終わります。`foundation/tests/cnet/test_demo.py` がこれを実行します。

## 別々の PC でつなぐ

`target/cnet-demo/A/config.yaml` の `udp` を次のように書き換えます。`peers` に相手の PC のアドレスを書くか、マルチキャストを使います。

```yaml
    udp:
      bind: "0.0.0.0:32032"
      multicast: "239.255.32.32:32032"
```

ノード番号が重ならないように `expansion_card[].network.node` を変えるか、省略して自動取得にします。
