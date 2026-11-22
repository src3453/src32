# CNET TCP ブリッジ（telnet／SSH）仕様

> **状態: 第 2 版を実装済み。** ブリッジは `tools/cnet-bridge/cnet_bridge.py`（CNET と L3P は `tools/l3p/l3p.py`）、Foundation のクライアントは `foundation/bin/telnet.c`（`telnet` と `ssh`、WAL `l3p` を使う）、テストは `tools/cnet-bridge/test_cnet_bridge.py` と `foundation/tests/cnet/test_telnet.py` です。第 2 版（2026-11-23）で、第 1 版の独自ストリーム（PROTO `0x40`）を [L3P](spec_NET_L3P.md) のストリームに置き換え、`0x40` は廃止しました。両端を同時に切り替えたので、第 1 版との互換はありません。

## 1. 目的と方針

CNET（[spec_NET.md](spec_NET.md)）はデータグラムだけを運び、TCP/IP とは互換ではありません。このブリッジは、ホストで動く Python プログラムが CNET セグメント上の 1 ノードとして振る舞い、CPT32 から外部の telnet サーバーや SSH サーバーへの接続を中継します（spec_NET §10 の「TCP/IP ブリッジ」）。

```
Foundation の telnet / ssh ──L3P のストリーム（§3）── ブリッジ ──TCP── telnet サーバー
                                                                         └──SSH── SSH サーバー
```

- CNET 側は常に **telnet プロトコル**（RFC 854）です。Foundation のクライアントは telnet クライアントだけを持ちます。
- `telnet://` の接続では、ブリッジはバイト列をそのまま TCP へ中継します（telnet ↔ telnet）。
- `ssh://` の接続では、ブリッジが telnet サーバーとして振る舞い、SSH クライアントとして相手に接続します（telnet ↔ SSH）。ログイン名・パスワード・ホスト鍵の確認は、telnet のストリーム上の対話で行います（§5）。
- カード、ホストの物理層、Foundation のカーネルドライバは変えません。ストリームの信頼性（再送・順序・流量制御）は L3P（spec_NET_L3P §6）が受け持ちます。
- 外部への接続先の制限と許可はブリッジの責任です（§6）。

## 2. アドレスと既知の値

| 項目 | 値 |
|---|---|
| ブリッジのノード | 既定 `0xFF01`（予約ノード `0xFF00–0xFFFE`、spec_NET §2）。`--node` で変更できる |
| ブリッジのソケット | `23` |
| PROTO | `0x04`（L3P）。L3P のストリーム（KIND 2） |
| クライアントのソケット | 動的割当（`128–254`） |

ブリッジは自分のノードへの PROBE に IN_USE を返し、ソケット 4 のエコー（spec_NET §4.3）にも応答します。`cnet ping .FF01` で到達を確かめられます。

ブリッジは DST_NET が 0 か自分のネットワーク番号（`--net`、既定 0）のデータグラムを受け付け、応答の DST_NET には要求の SRC_NET を使います。クライアントは、応答の SRC_NET が 0 か自分が指定した NET のどちらでも同じ相手として扱います。

## 3. 接続（L3P のストリーム）

1 本の接続は、クライアントがブリッジのソケット 23 へ開く L3P のストリームの結合（spec_NET_L3P §4・§6）です。再送・順序・流量制御・キープアライブ・無通信タイムアウトは L3P に従います。

1. クライアントは結合を開き、最初に接続先 URI（§4、UTF-8、255 byte まで）と LF（`0x0A`）を送ります。CR は付けてもかまいません（ブリッジが取り除きます）。
2. ブリッジは URI を読み、接続先に接続できたら `OK` と LF の 1 行を返します。`telnet://` では TCP 接続が成立してから、`ssh://` ではすぐに返します（SSH の接続と認証はストリーム上で行うため）。クライアントはこの行を表示せず、その後のバイト列から telnet として扱います。
3. 断るときは、ブリッジは結合を RESET します。理由（表示用の文）は RESET の MSG に入れ、クライアントがそれを表示します。

| 場合 | RESET の REASON |
|---|---|
| URI がない（30 秒以内に LF が来ない、255 byte を超える、行の前に FIN）・形式が不正・`--allow` で許していない | 3 REFUSED |
| 接続先への接続の失敗、ブリッジ内部の誤り | 9 ABORT |
| 同時接続数の上限（`--max-sessions`） | 2 BUSY（L3P が OPEN に返す） |

4. 接続先が閉じたら、ブリッジは FIN を送ります。クライアントは両方向の FIN の後に CLOSE を送って結合を終えます（spec_NET_L3P §4.4）。クライアントが強制終了されて（シェルの Ctrl+C など）結合が残った場合は、L3P の無通信タイムアウト（120 秒）でブリッジが片付けます。

第 1 版との違い: 接続先は OPEN の DATA ではなくストリームの最初の行で送り、ACCEPT の代わりに `OK` の行で接続の成立を知らせます。

## 4. 接続先 URI

| 形式 | 意味 |
|---|---|
| `telnet://HOST[:PORT]` | TCP で HOST:PORT（既定 23）へ接続し、バイト列を中継する |
| `ssh://[USER@]HOST[:PORT]` | SSH（既定 22）で接続し、端末つきのシェルを開く。USER を省くとログイン名を尋ねる |

HOST は名前、IPv4、または `[...]` で囲んだ IPv6 です。

## 5. SSH モードの telnet 側

ブリッジは接続直後に `IAC WILL ECHO`、`IAC WILL SGA`、`IAC DO NAWS`、`IAC DO TTYPE` を送ります。

- NAWS（RFC 1073）の窓サイズを SSH の PTY に反映します（後から変わったら `window-change`）。
- TTYPE（RFC 1091）の端末名を PTY の TERM にします。届かなければ `ansi` です。
- クライアントからの CR LF と CR NUL は CR 1 byte にしてから SSH へ渡します。SSH からの `0xFF` は `IAC IAC` にします。
- ホスト鍵は、ブリッジの known_hosts（`--known-hosts`、既定 `~/.cpt32/cnet_bridge_known_hosts`）とユーザーの `~/.ssh/known_hosts` で確かめます。未知の鍵は、指紋を表示して `yes` の入力で受け入れ、ブリッジの known_hosts に書きます。登録済みと異なる鍵なら接続を中止します。
- 認証は、ssh-agent と `--identity` の鍵、既定の鍵ファイル、パスワード（3 回まで）、keyboard-interactive の順に試します。パスワードは telnet の対話で尋ね、エコーしません。ブリッジはパスワードを保存もログ出力もしません。
- SSH のシェルが終わったら、ブリッジは FIN を送ります。

## 6. 許可と安全

- ブリッジが接続してよい宛先は、起動時の `--allow PATTERN`（複数可）で指定します。PATTERN は `HOST`、`HOST:PORT`、`*` を含むワイルドカード（`*.example.com:22`）です。`--allow-any` はすべてを許可します。どれにも合わない接続先は RESET（REFUSED、`not allowed`）です。何も指定しなければ、どこにも接続しません。
- 名前の照合は URI に書かれた HOST の文字列で行います（解決後の IP ではありません）。
- CNET セグメントの参加者は誰でもブリッジに結合を開けます。マルチキャストで LAN に開いたセグメントにブリッジを置くときは、`--allow` を必要な宛先だけに絞ってください。
- 同時接続数は `--max-sessions`（既定 8）までです。超えた OPEN は L3P の RESET（BUSY）です。

## 7. Foundation のクライアント

`/bin/telnet.wax` と `/bin/ssh.wax`（同じソース `foundation/bin/telnet.c`）です。使い方は [シェル仕様](spec_Foundation_shell.md) のコマンド表を参照してください。

- ブリッジの CNET アドレスは `-b ADDR[:SOCK]`（spec_NET §2 の `NET.NODE:SOCK`）で指定し、既定は `.FF01:17` です。SOCK は 16 進で、省くとソケット 23（`17`）です（シェルの変数は子プロセスに渡らないため、環境変数では指定しません）。
- コンソールを RAW にし、キーを VT100 系の byte 列へ変換して送ります（矢印は `ESC [ A` など、Backspace は DEL、Enter は CR NUL、Ctrl+文字は C0）。受信は IAC を処理して画面へ書きます。UTF-8 の文字が DATA の境目で分かれたら、残りが届くまで書くのを待ちます。
- 対応するオプションは ECHO・SGA・BINARY（相手が WILL なら DO）と NAWS・TTYPE・BINARY（相手が DO なら WILL）です。他は断ります。相手が ECHO を WILL しない間は、入力を自分で表示します。端末名は既定 `ansi`（`-t TERM` で変更）。Foundation のコンソールはスクロール領域や行挿入を持たないので（API §8.1.1）、全画面のアプリは崩れることがあります。
- シェルは実行中のコマンドの Ctrl+C を中断に使うため（シェル仕様 §9.3）、Ctrl+C は相手に届かずクライアントが終了します。エスケープキー Ctrl+] の後に `c` で Ctrl+C を送り、`q` で接続を閉じて終わり、`]` で Ctrl+] 自体を送ります。
