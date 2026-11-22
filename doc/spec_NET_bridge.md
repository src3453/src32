# CNET TCP ブリッジ（telnet／SSH）仕様

> **状態: 第 1 版を実装済み。** ブリッジは `tools/cnet-bridge/cnet_bridge.py`、Foundation のクライアントは `foundation/bin/telnet.c`（`telnet` と `ssh`）、テストは `tools/cnet-bridge/test_cnet_bridge.py` と `foundation/tests/cnet/test_telnet.py` です。

## 1. 目的と方針

CNET（[spec_NET.md](spec_NET.md)）はデータグラムだけを運び、TCP/IP とは互換ではありません。このブリッジは、ホストで動く Python プログラムが CNET セグメント上の 1 ノードとして振る舞い、CPT32 から外部の telnet サーバーや SSH サーバーへの接続を中継します（spec_NET §10 の「TCP/IP ブリッジ」）。

```
Foundation の telnet / ssh ──CNET（PROTO 0x40 のストリーム、§3）── ブリッジ ──TCP── telnet サーバー
                                                                         └──SSH── SSH サーバー
```

- CNET 側は常に **telnet プロトコル**（RFC 854）です。Foundation のクライアントは telnet クライアントだけを持ちます。
- `telnet://` の接続では、ブリッジはバイト列をそのまま TCP へ中継します（telnet ↔ telnet）。
- `ssh://` の接続では、ブリッジが telnet サーバーとして振る舞い、SSH クライアントとして相手に接続します（telnet ↔ SSH）。ログイン名・パスワード・ホスト鍵の確認は、telnet のストリーム上の対話で行います（§5）。
- カード、ホストの物理層、Foundation のカーネルドライバは変えません。ストリームの信頼性（再送・順序・流量制御）は、ブリッジとクライアントの両端で実装します。
- 外部への接続先の制限と許可はブリッジの責任です（§6）。

## 2. アドレスと既知の値

| 項目 | 値 |
|---|---|
| ブリッジのノード | 既定 `0xFF01`（予約ノード `0xFF00–0xFFFE`、spec_NET §2）。`--node` で変更できる |
| ブリッジのソケット | `23` |
| PROTO | `0x40`（STREAM。予約域 `0x40–0x7F` の最初の値、spec_NET §4.2） |
| クライアントのソケット | 動的割当（`128–254`） |

ブリッジは自分のノードへの PROBE に IN_USE を返し、ソケット 4 のエコー（spec_NET §4.3）にも応答します。`cnet ping .FF01` で到達を確かめられます。

ブリッジは DST_NET が 0 か自分のネットワーク番号（`--net`、既定 0）のデータグラムを受け付け、応答の DST_NET には要求の SRC_NET を使います。クライアントは、応答の SRC_NET が 0 か自分が指定した NET のどちらでも同じ相手として扱います。

## 3. ストリームプロトコル（PROTO 0x40）

1 本の接続は、クライアントのアドレス・ソケットと、クライアントが選ぶ 16 bit の CONN（0 以外）で識別します。データグラムのペイロードは次の 16 byte のヘッダとデータです。数値は big-endian です。

| Offset | Size | Field | 説明 |
|---|---|---|---|
| 0 | 1 | TYPE | 下表 |
| 1 | 1 | FLAGS | bit0 FIN（DATA だけ）。他は 0 |
| 2 | 2 | CONN | 接続 ID |
| 4 | 4 | SEQ | DATA: 先頭 byte の番号。その他は 0 |
| 8 | 4 | ACK | 次に受け取りたい byte の番号（OPEN 以外で有効） |
| 12 | 2 | WINDOW | ACK から先に受け取れる byte 数 |
| 14 | 2 | RSV | 0 |
| 16 | n | DATA | 最大 1008 byte（CNET の MTU 1024 − 16） |

| TYPE | 名前 | 向き | 内容 |
|---|---|---|---|
| `0x01` | OPEN | C→B | DATA = 接続先 URI（UTF-8、§4） |
| `0x02` | ACCEPT | B→C | 接続を受け付けた。DATA なし |
| `0x03` | DATA | 両方 | ストリームのデータ。FLAGS.FIN で送信の終わり |
| `0x04` | ACK | 両方 | 受信の確認だけ。DATA なし。ACK には応答しない |
| `0x05` | RESET | 両方 | 接続の中止。DATA = 理由（UTF-8、表示用）。応答しない |

### 3.1 接続

1. クライアントは OPEN を送り、ACCEPT か RESET を待ちます。応答がなければ 1 秒ごとに同じ OPEN を送り直し、15 秒で諦めます。
2. ブリッジは接続を受け付けたら ACCEPT を返します。`telnet://` では TCP 接続が成立してから、`ssh://` ではすぐに返します（SSH の接続と認証はストリーム上で行うため）。拒否・接続失敗は RESET で、理由をクライアントが表示します。
3. 同じクライアント・ソケット・CONN の OPEN が重なったら（再送）、ブリッジは新しい接続を作らず、確立済みなら ACCEPT を返し直します。同じクライアント・ソケットで別の CONN の OPEN が来たら、古い接続を捨てて新しく始めます。
4. 両方向とも SEQ は 0 から始まります。

### 3.2 データと確認

- 送信側は、相手の ACK と WINDOW が許す範囲で DATA を送ります。確認されていないデータは保持し、一定時間（クライアント 300 ms、ブリッジ 300 ms）ACK が進まなければ、確認されていない先頭から送り直します（go-back-N）。
- 受信側は、SEQ が次に期待する番号と一致する DATA だけを受け取り、それ以外は捨てます。DATA を受け取るたびに（捨てた場合も、長さ 0 でも）ACK を返します。自分の DATA を送るときは、その ACK 欄で兼ねてかまいません。
- FIN は 1 byte 分の番号を使います。FIN 付き DATA のデータの後の番号が確認されたら、その方向の送信は終わりです。両方向が終わったら接続を閉じます。
- クライアントの WINDOW は 4032 byte（1008 byte × 4）を上限にします。カードのソケットの受信キューは 8 データグラムなので（spec_NET §6.2）、ブリッジは WINDOW に加えて、確認されていない DATA を 4 データグラムまでに抑えます。
- ブリッジは、15 秒間なにも受け取らない接続に長さ 0 の DATA（キープアライブ）を送ります。再送を 30 回続けても ACK が進まなければ、RESET を送って接続を捨てます。クライアントが強制終了されて（シェルの Ctrl+C など）ソケットが閉じた場合も、これで片付きます。
- 知らない接続への DATA・ACK には、ブリッジは RESET を返します。

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

- ブリッジが接続してよい宛先は、起動時の `--allow PATTERN`（複数可）で指定します。PATTERN は `HOST`、`HOST:PORT`、`*` を含むワイルドカード（`*.example.com:22`）です。`--allow-any` はすべてを許可します。どれにも合わない OPEN は RESET（`not allowed`）です。何も指定しなければ、どこにも接続しません。
- 名前の照合は URI に書かれた HOST の文字列で行います（解決後の IP ではありません）。
- CNET セグメントの参加者は誰でもブリッジに OPEN を送れます。マルチキャストで LAN に開いたセグメントにブリッジを置くときは、`--allow` を必要な宛先だけに絞ってください。
- 同時接続数は `--max-sessions`（既定 8）までです。超えた OPEN は RESET（`busy`）です。

## 7. Foundation のクライアント

`/bin/telnet.wax` と `/bin/ssh.wax`（同じソース `foundation/bin/telnet.c`）です。使い方は [シェル仕様](spec_Foundation_shell.md) のコマンド表を参照してください。

- ブリッジの CNET アドレスは `-b ADDR[:SOCK]`（spec_NET §2 の `NET.NODE:SOCK`）で指定し、既定は `.FF01:17` です。SOCK は 16 進で、省くとソケット 23（`17`）です（シェルの変数は子プロセスに渡らないため、環境変数では指定しません）。
- コンソールを RAW にし、キーを VT100 系の byte 列へ変換して送ります（矢印は `ESC [ A` など、Backspace は DEL、Enter は CR NUL、Ctrl+文字は C0）。受信は IAC を処理して画面へ書きます。UTF-8 の文字が DATA の境目で分かれたら、残りが届くまで書くのを待ちます。
- 対応するオプションは ECHO・SGA・BINARY（相手が WILL なら DO）と NAWS・TTYPE・BINARY（相手が DO なら WILL）です。他は断ります。相手が ECHO を WILL しない間は、入力を自分で表示します。端末名は既定 `ansi`（`-t TERM` で変更）。Foundation のコンソールはスクロール領域や行挿入を持たないので（API §8.1.1）、全画面のアプリは崩れることがあります。
- シェルは実行中のコマンドの Ctrl+C を中断に使うため（シェル仕様 §9.3）、Ctrl+C は相手に届かずクライアントが終了します。エスケープキー Ctrl+] の後に `c` で Ctrl+C を送り、`q` で接続を閉じて終わり、`]` で Ctrl+] 自体を送ります。
