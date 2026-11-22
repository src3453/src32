# CFP server

ホストのディレクトリや FS2 イメージを、CNET の上の CFP（CNET Filing Protocol、[仕様](../../doc/spec_NET_FP.md)）で輸出するサーバーです。CNET の UDP セグメントに 1 ノード（既定 `FF02`）として参加し、L3P（[仕様](../../doc/spec_NET_L3P.md)、`tools/l3p/l3p.py`）のトランザクションをソケット 9 で受け付けます。

```powershell
python tools/cfp-server/cfp_server.py --bind 127.0.0.1:32041 --peer 127.0.0.1:32032 --net 1 --export share=C:\cpt32\share --export disk=build\foundation.fs2,ro
```

- `--export NAME=PATH[,ro]`: PATH がディレクトリならホストのディレクトリ、ファイルなら FS2 イメージ。`ro` で読み取り専用。
- セグメントへの接続（`--bind`、`--multicast`、`--peer`、`--segment-id`、`--net`、`--node`）は TCP ブリッジと同じ意味です。エミュレータの `config.yaml` の `udp.peers` にサーバーのポートを入れてください。
- `--allow ADDR`／`--allow-any` で送信元を限ります。何も指定しなければ同じネットワークの全ノードを許します。
- 暗号化（`--require-encryption`、`--allow-key FINGERPRINT`、`--key PATH`）には `cryptography` が必要です。起動時にサーバーの鍵の指紋を表示します。
- FS2 イメージは、構造の変更ごと、書き込みは CLUNK・SYNC・1 秒後に、`tools/fs2` の検証を通してから書き戻します。エミュレータの IDC が同じイメージを使っている間は輸出しないでください。

Foundation からは `netfs exports`、`netfs mount .FF02 share s` で `/net/s` に mount して使います（[CFP 仕様 §11](../../doc/spec_NET_FP.md)、`foundation/tests/cfp`）。Python のクライアント `CfpClient` と `list_exports` が `cfp_server.py` にあります。

テスト: `python -m unittest discover -s tools/cfp-server -v`（約 7 分。20% の損失を入れた転送を含みます）。
