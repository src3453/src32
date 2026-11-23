# CNET TCP ブリッジ（telnet／SSH）

CPT32 の CNET セグメント上に 1 ノードとして参加し、Foundation の `telnet`／`ssh` から外部の telnet サーバー・SSH サーバーへの接続を中継するホストプログラムです。プロトコルと安全上の約束は [doc/spec_NET_bridge.md](../../doc/spec_NET_bridge.md) にあります。

- `telnet://`: バイト列をそのまま TCP へ中継します（telnet ↔ telnet）。
- `ssh://`: ブリッジが telnet サーバー兼 SSH クライアントになります（telnet ↔ SSH）。ホスト鍵の確認、ログイン名、パスワードは Foundation の画面で対話します。[paramiko](https://www.paramiko.org/) が必要です（`pip install -r tools/cnet-bridge/requirements.txt`）。

## 使い方

エミュレータの `config.yaml` で CNET カードを UDP セグメントにつなぎ、ブリッジのポートを `peers` に入れます。

```yaml
network_segment:
  - name: lan
    network: 1
    udp:
      bind: "127.0.0.1:32032"
      peers: ["127.0.0.1:32040"]   # ブリッジ
expansion_card:
  - slot: 2
    package: "plugins/cnet.cpt32card"
    network: { segment: lan }
```

ブリッジを起動します。接続してよい宛先を `--allow` で必ず指定します（指定がなければすべて拒否）。

```powershell
python tools/cnet-bridge/cnet_bridge.py --bind 127.0.0.1:32040 --peer 127.0.0.1:32032 --net 1 --allow localhost --allow "*.example.org:22"
```

Foundation のシェルから:

```
/> cnet ping .65281
/> telnet towel.blinkenlights.nl
/> ssh me@server.example.org
/> ssh -p 2222 -b 1.65281 server.example.org
```

接続中の Ctrl+C はシェルがコマンドの中断に使うため、相手には届きません。Ctrl+] の後に `c` で Ctrl+C を送り、`q` で接続を閉じます。

| オプション | 意味 |
|---|---|
| `--bind HOST:PORT` | セグメントの UDP アドレス（既定 `0.0.0.0:32032`） |
| `--multicast GROUP:PORT` | セグメントのマルチキャストグループ（エミュレータの `udp.multicast` と同じ値） |
| `--peer HOST:PORT` | ユニキャストの相手（複数可）。マルチキャストを使わないときは、届いたフレームの送信元も相手として覚える |
| `--segment-id N` | `udp.segment_id` と同じ値（既定 0） |
| `--net N` | セグメントのネットワーク番号（`network_segment[].network`、既定 0） |
| `--node N` | ブリッジのノード番号（10 進、末尾に `H` で 16 進。既定 `65281`） |
| `--allow PATTERN` | 接続を許す宛先。`HOST`、`HOST:PORT`、ワイルドカード（複数可） |
| `--allow-any` | すべての宛先を許す |
| `--max-sessions N` | 同時接続数（既定 8） |
| `--known-hosts PATH` | ブリッジで受け入れた SSH ホスト鍵（既定 `~/.cpt32/cnet_bridge_known_hosts`）。`~/.ssh/known_hosts` も読む |
| `--identity PATH` | SSH の秘密鍵（複数可）。ssh-agent と `~/.ssh/id_*` も試す |

## テスト

```powershell
python -m unittest discover -s tools/cnet-bridge -v
python -m unittest discover -s foundation/tests/cnet -p test_telnet.py -v
```

前者は Python の代役クライアント（パケット損失あり）でブリッジを確かめ、後者は testbench 上の Foundation の `telnet`／`ssh` からブリッジ経由で接続します（testbench・Clang・Cargo の wasm32 ターゲットが必要）。SSH のテストは paramiko がなければ skip します。
