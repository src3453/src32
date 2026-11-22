# CNET TCP ブリッジ（telnet／SSH）と Foundation の telnet・ssh

- 仕様は `doc/spec_NET_bridge.md`。ブリッジはホストの Python（`tools/cnet-bridge/cnet_bridge.py`）で、CNET の UDP カプセル化（spec_NET §8.3）を自分で話す 1 ノード（既定 `FF01`、ソケット 23）。カード・物理層・カーネルドライバは変えていない（spec_NET §10 の「ブリッジはセグメント上のノード」の方針どおり）。
- CNET はデータグラムだけなので、信頼性のあるストリーム（PROTO `0x40`、16 byte ヘッダ、byte 単位の SEQ/ACK、go-back-N、FIN は 1 番号を消費）を両端で実装した。カードの受信キューはソケットあたり 8 データグラムなので、ブリッジは WINDOW に加えて未確認 DATA を 4 データグラム（4032 byte）に抑える。これを外すと Foundation 側で RX_DROPPED が出て再送だらけになる。
- CNET 側は常に telnet。`telnet://` はバイト列を素通し（IAC も Foundation のクライアントが処理）、`ssh://` はブリッジが telnet サーバー（WILL ECHO/SGA、DO NAWS/TTYPE、CR LF・CR NUL → CR、出力の 0xFF は IAC IAC）兼 paramiko の SSH クライアント。ホスト鍵の確認・ログイン・パスワードはストリーム上の対話（`Dialog`）で、ブリッジがエコーする。
- `foundation/bin/ssh.c` は `#define TELNET_SSH 1` と `#include "telnet.c"` だけ。artifact cache は depfile で依存を追うので、telnet.c を変えると ssh も作り直される。`build.py` の `COMMANDS` と `CNET_COMMANDS` に両方を足した。
- **シェルの変数は子プロセスに渡らない**（shell spec §5.1、shell.c は envp を渡さない）。`fnd_env` は存在するが、シェルから起動したコマンドでは空。ブリッジのアドレスは `-b` だけにした。
- **Ctrl+C はシェルが常に奪う**（break スレッドが `CON_BREAK_WAIT` を待ち続け、子を kill する）。端末クライアントへ Ctrl+C を渡す仕組みはないので、Ctrl+] → `c` で送る。kill されたクライアントの接続は、ブリッジのキープアライブ（15 秒）と 60 秒の無応答で片付く。Ctrl+C を通す console／shell の仕組みは未実装（入れるなら API §8.4 とシェル §9.3 の変更）。
- クライアントは OPENING 中に DATA が届いたら（ACCEPT が落ちた場合）接続済みとみなす。`Connected.` の表示はその遷移の瞬間に出す（後で出すと、先に届いたバナーの後ろに表示が割り込む）。
- Python の heredoc で C や Python のソースを書き換えると `\n`・`\r` のエスケープが実文字になって壊れる。ソース中のエスケープを含む置換は Edit ツールで行うこと。
- testbench のテスト（`foundation/tests/cnet/test_telnet.py`）は test_shell の `--stop-ticks 800` のイメージを使うので、カーネルは約 frame 480 で止まる。`--type` の最後の入力（`echo status=$?`）はそれより前に置く。仮想時間と実時間は同期していないが、ブリッジの再送は実時間、クライアントの再送は tick で、重複はストリームが捨てるので問題ない。
- 検証（2026-11-22、Windows、clang 20 + llvmlite 22）: `python -m unittest discover -s tools/cnet-bridge -v`（13 件、20% 損失のバルク転送と paramiko の SSH を含む）、`python -m unittest discover -s foundation/tests/cnet -p test_telnet.py -v`（4 件、VM と `FND_MM=nommu`）、`foundation/tests/cnet` の既存テスト（7 件）。`foundation/tests/shell` は 30/31 で、`test_line_editing_and_history`（Ctrl+C の型先行入力）だけ失敗したが、telnet・ssh を外したビルドでも同じく失敗するので、この作業とは無関係（作業ツリーに未コミットの `tools/llvm-src32/backend.py` の変更があった）。
- 未確認: GUI のエミュレータでの実操作、実在の telnetd／OpenSSH サーバーへの接続（テストは Python の代役サーバー）、マルチキャストのセグメントでのブリッジ。Foundation のコンソールはスクロール領域・行挿入を持たないので、vim などの全画面アプリは崩れる（TERM 既定 `ansi`）。
