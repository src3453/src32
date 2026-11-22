# Foundation の WAL l3p 1.0 と、TCP ブリッジの L3P への移行

- 日付: 2026-11-23
- Issue: なし（ユーザー指示で作成せず）
- 仕様: `doc/spec_NET_L3P.md` §10.1、`doc/spec_NET_bridge.md` 第 2 版、`doc/spec_Foundation_API.md` §3（-73 `FND_E_STALE`）と §9.6（libl3p）。

## 決めたこと・落とし穴

- **プロセスの領域は 8 個**（`WAX_REGIONS`、Extension P）。image 1 つが RX と RW の 2 領域を使うので、`fnd`・`cnet`・`l3p` を import する telnet は exec で `too many regions (-15)` になった（VM でも同じ。launch の計画が共通のため）。そこで libl3p は libcnet のコード（`lib/cnet/cnet.c`）を静的に含めて `fnd` だけを import し、telnet／ssh も `cnet.c` を一緒にリンクして `fnd`＋`l3p` だけを import する。WAL を増やすときは、利用側の import 数×2＋スタックを数えること。
- libl3p 1.0 は開始側だけ（トランザクション、ストリーム、UREQ）。待ち受けと暗号化は未実装で、`L3P_OPT_ENC` は UNSUPPORTED。結合ごとに動的ソケットを 1 個使う（CONN の多重化はしない）。状態は呼び出し側の `l3p_assoc`（約 9 KiB）。WAL は関数ポインタを渡せないので、`l3p_ucall` は結果をバッファに並べる。
- ブリッジ第 2 版: クライアントは L3P のストリームを開き、接続先 URI と LF を最初の行で送る。ブリッジは接続できたら `OK` LF を返し、断るときは RESET（REFUSED／ABORT）の MSG に理由を入れる。PROTO `0x40` は廃止（両端を同時に切り替えた）。`Assoc.abort(message, reason)` で REASON を選べるようにした。
- ブリッジは `tools/l3p/l3p.py` の Segment／Node／Endpoint を使い、自前の CNET 実装を消した。セッションの終わりの CLOSE はクライアント（開始側）が送り、ブリッジは待つだけ（強制終了されたクライアントの結合は L3P の無通信タイムアウト 120 秒で消える。第 1 版は 60 秒）。
- Bash ツールの heredoc（`<<'EOF'` でも）で Python を流すと、`\n` や `\x01` などのエスケープが壊れることがある。エスケープを含む置換は Write ツールでスクリプトをファイルに書いて実行するか、Edit ツールを使う（cnet-bridge のメモと同じ注意）。

## テスト（2026-11-23、Windows、clang 20）

- `python -m unittest discover -s tools/cnet-bridge -v`（14 件、L3P のクライアントで。20% 損失、SSH、URI の行がないときの REFUSED、知らない CONN への RESET）。
- `python -m unittest discover -s foundation/tests/cnet -p test_telnet.py -v`（4 件）と `FND_MM=nommu` の同じテスト、`foundation/tests/cnet` の他のテスト（9 件）。
- `foundation.tests.disk.test_disk_boot.DiskBootTests.test_image_layout`（`/lib/cnet.1.wal` と `/lib/l3p.1.wal` が既定のディスクにあること）。
- 未確認: libl3p のトランザクションは Foundation 上ではまだ動かしていない（netfs で使う予定）。ブロードキャストの `l3p_ucall` も同じ。シェルのテスト（`foundation/tests/shell`）は今回は走らせていない。
