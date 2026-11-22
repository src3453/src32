# CNET Filing Protocol（CFP）仕様

> **状態: ドラフト（未実装）。** 2026-11-22 に起草しました。バイト配置・番号・時間の具体値は提案で、§13 の「確認が必要な決定」が決まるまでは変更できます。実装したら、実装場所とテストをこの欄に書きます。

## 1. 目的と方針

CNET（[spec_NET.md](spec_NET.md)）の上で、ファイルとディレクトリを共有するシンプルなプロトコルを定めます。Plan 9 の 9P と同じく、クライアントが選んだ番号（fid）でサーバー上のファイルを指し、walk・open・read・write・clunk の小さな操作の組み合わせですべてを表します。

### 1.1 決定事項（提案）

| 項目 | 決定 |
|---|---|
| 名前 | CNET Filing Protocol。略称は **CFP**（本書、ソース、コマンドの名前ではこの略称を使う） |
| 下位層 | [L3P](spec_NET_L3P.md) のトランザクション（§2）。再送・重複除去・順序・暗号化は L3P に任せる |
| 1 メッセージ | 1 つの L3P トランザクションに 1 メッセージ。分割・再組み立てはしない。BODY は最大 980 byte |
| 操作の考え方 | 9P2000 に倣う fid 方式。パス文字列ではなく、成分ごとの walk でたどる |
| 意味論 | Foundation の fs（[API §5〜§7](spec_Foundation_API.md)、[FS2 §6〜§11](spec_FS2.md)）。エラー番号、`fnd_stat`、mode、flags、時刻の形式をそのまま運ぶ |
| 認証 | L3P の暗号化と静的鍵による（オプション）。ほかに、輸出（export）ごとの読み取り専用指定と、接続を許す CNET アドレス・鍵の一覧で制限する（§10） |
| サーバー | 第 1 版は、telnet の TCP ブリッジ（[spec_NET_bridge.md](spec_NET_bridge.md)）と同じく、ホストの Python プログラム `tools/cfp-server/cfp_server.py` として作る。CNET セグメント上の 1 ノードとして振る舞い、ホストのディレクトリか FS2 イメージを輸出する（§9）。カード・ホストの物理層・カーネルドライバは変えない。Foundation 上のサーバーは将来 |
| クライアント | Foundation のユーザーサービス `netfs`。`/net/<名前>` に mount する（§11） |

### 1.2 9P との違い

- 信頼できるストリームではなく、L3P のトランザクション（要求ごとにちょうど 1 回、順序どおり）の上に載ります。
- メッセージは 1 パケットに収まる大きさに限ります（msize の交渉はありません）。
- walk が途中で失敗したときに、失敗した成分のエラーを返します（Foundation の NOT_FOUND／ACCESS／NOT_DIR を区別するため）。
- ディレクトリの列挙は read ではなく専用の READDIR で、位置は不透明な cookie です。
- 改名は親ディレクトリをまたげます（Foundation の `FS_RENAME` と同じ）。
- `Tflush`（要求の取り消し）と `Tauth` はありません。

## 2. 下位層（L3P）

CFP は [L3P](spec_NET_L3P.md) のトランザクション（spec_NET_L3P §5）の上で動きます。再送、重複除去、順序、暗号化は L3P が受け持ち、CFP はそれを前提にします。

| 項目 | 値 |
|---|---|
| L3P のサービス | トランザクション（KIND 1） |
| サーバーのソケット | `9`（spec_NET_L3P §2） |
| クライアントのソケット | 動的割当（`128–254`） |
| ホストサーバーの既定ノード | `0xFF02`（予約ノード、spec_NET §2。ブリッジは `0xFF01`）。`--node` で変えられ（§9.1）、通常のノードでサーバーを動かしてもよい |

- **1 つの L3P の結合が 1 つの CFP セッションです。** fid の表は結合ごとにあり、結合が閉じるか壊れると（spec_NET_L3P §4.3・§4.4）、サーバーはその結合の fid をすべて CLUNK と同じく片付けます（書き込み中のバッファは書き出す）。
- L3P が各要求をちょうど 1 回、送った順にサーバーへ渡すので、非冪等な操作（CREATE・REMOVE・RENAME・WRITE の APPEND など）も 1 回だけ実行されます。クライアントは窓（最大 4 要求）の範囲で、応答を待たずに続けて要求を送れます。
- 暗号化（spec_NET_L3P §7）はクライアントが結合ごとに選びます。サーバーは必須にできます（§9.1 `--require-encryption`）。netfs は mount 名でサーバーの静的鍵を覚えます（spec_NET_L3P §7.4）。
- 輸出の一覧（EXPORTS、§5）だけは結合を作らず、L3P の結合なしのトランザクション（UREQ、spec_NET_L3P §5.5）で問い合わせます。
- 要求の打ち切り（15 秒）で結合が壊れたら、クライアントはそのセッションの fid をすべて失います。打ち切った要求が実行されたかどうかは分かりません（spec_NET_L3P §5.4）。

## 3. メッセージ

1 つの CFP メッセージは、1 つの L3P のトランザクションの要求か応答です。

### 3.1 共通ヘッダ（4 byte）

すべての数値は big-endian です。

| Offset | Size | Field | 説明 |
|---|---|---|---|
| 0 | 1 | TYPE | §3.3。応答は要求の TYPE に `0x80` を足した値、エラー応答は `0xFF` |
| 1 | 1 | FLAGS | 0（Rexports だけ §5） |
| 2 | 2 | RSV | 0 |
| 4 | n | BODY | 最大 980 byte（L3P の暗号化したパケットの上限 984 byte からヘッダを引いた値。暗号化の有無によらず同じ上限にする） |

- サーバーは、RSV が 0 でないもの、BODY の長さが TYPE の形式と合わないものに `Rerror(BAD_ARG)` を返します。
- 予約欄は 0 で送り、受信側は 0 でなければ `BAD_ARG` にします。

### 3.2 共通の型

| 型 | 内容 |
|---|---|
| `fid` | u32。クライアントが選ぶ。`0xFFFFFFFF` は NOFID |
| `name` | u16 の byte 数 + UTF-8（NUL なし）。1〜900 byte（§3.4） |
| `qid` | 16 byte: TYPE u8（1 FILE、2 DIR）、RSV u8×3、GEN u32、PATH u64。サーバー上のファイルの同一性。PATH が同じでも GEN が違えば別物（FS2 §11.2 のハンドル同一性） |
| `stat` | `fnd_stat` 64 byte（API §10.1 と同じ配置） |
| `volinfo` | `fnd_volinfo` 32 byte（API §10.3 と同じ配置） |
| `status` | i32。Foundation のエラー番号（§7） |

### 3.3 TYPE 一覧

| T | R | 名前 | L3P | 内容 |
|---|---|---|---|---|
| `0x01` | `0x81` | HELLO | 結合 | セッションの始め（§4） |
| `0x02` | `0x82` | EXPORTS | 結合なし | 輸出の一覧（§5）。ブロードキャストで探せる |
| `0x10` | `0x90` | ATTACH | 結合 | 輸出のルートを fid に結び付ける |
| `0x11` | `0x91` | WALK | 結合 | 名前をたどって新しい fid を作る |
| `0x12` | `0x92` | CLUNK | 結合 | fid を捨てる |
| `0x13` | `0x93` | OPEN | 結合 | fid を開く |
| `0x14` | `0x94` | CREATE | 結合 | ファイル／ディレクトリを作る |
| `0x15` | `0x95` | READ | 結合 | 読む |
| `0x16` | `0x96` | WRITE | 結合 | 書く |
| `0x17` | `0x97` | READDIR | 結合 | ディレクトリを列挙する |
| `0x18` | `0x98` | STAT | 結合 | 属性を得る |
| `0x19` | `0x99` | SETATTR | 結合 | mode・UID・大きさ・更新時刻を変える |
| `0x1A` | `0x9A` | REMOVE | 結合 | 削除する |
| `0x1B` | `0x9B` | RENAME | 結合 | 改名・移動する |
| `0x1C` | `0x9C` | SYNC | 結合 | 永続化する |
| `0x1D` | `0x9D` | STATFS | 結合 | ボリューム情報を得る |
| — | `0xFF` | ERROR | 両方 | エラー応答（§7） |

`0x7F` は ERROR と区別するため要求に使いません。その他の値は予約で、サーバーは `Rerror(UNSUPPORTED)` を返します。結合なしで EXPORTS 以外を受け取ったら `Rerror(BAD_ARG)`、結合の中で EXPORTS を受け取っても答えてかまいません。

セッションの終了は L3P の CLOSE、キープアライブは L3P が行うので、CFP にはありません。

### 3.4 大きさの上限

| 名前 | 値 | 理由 |
|---|---|---|
| BODY | 980 byte | §3.1 |
| IOUNIT | 960 byte | Twrite の BODY 固定部 16 byte を引いて 964 byte 以下にし、32 の倍数にそろえた値。READ も同じ |
| 名前 | 900 byte | Rreaddir の 1 エントリ（68 byte + 名前）が 1 応答に必ず入る大きさ |
| WALK の成分 | 16 個／回 | 9P と同じ |
| 同時の要求 | L3P の窓（最大 4） | spec_NET_L3P §5.3 |

FS2 の名前は最大 1024 byte です。901〜1024 byte の名前（4 byte の文字が 226 個以上など）は CFP では扱えず、`NAME_TOO_LONG` になります。READDIR ではそのエントリを返しません（§6.8）。

## 4. HELLO

クライアントは、結合を確立したら最初に HELLO を送ります。HELLO の前の他の要求は `BAD_ARG`、2 回目の HELLO は同じ応答を返します。

Thello:

| Offset | Size | Field | 説明 |
|---|---|---|---|
| 0 | 4 | VERSION | `1` |
| 4 | 2 | NAME_LEN | 0〜64 |
| 6 | 2 | RSV | 0 |
| 8 | n | NAME | クライアントの表示名（UTF-8、ログ用） |

Rhello:

| Offset | Size | Field | 説明 |
|---|---|---|---|
| 0 | 4 | VERSION | `1`。サーバーが VERSION を扱えなければ `Rerror(UNSUPPORTED)` |
| 4 | 4 | CAPS | 0（予約） |
| 8 | 2 | MAX_FIDS | このセッションで同時に持てる fid の数（16 以上） |
| 10 | 2 | IOUNIT | 960 |
| 12 | 4 | EPOCH | サーバーの起動ごとに変わる値 |

セッション数の上限は L3P の結合数の上限（RESET の BUSY）で表します。

## 5. EXPORTS

Texports（BODY は空）は L3P の UREQ で、ユニキャストでもブロードキャスト（NODE `0xFFFF`）でも送れます。

Rexports:

| Offset | Size | Field | 説明 |
|---|---|---|---|
| 0 | 2 | COUNT | エントリ数 |
| 2 | 2 | RSV | 0 |
| 4 | … | エントリ | FLAGS u16（bit0 READ_ONLY、bit1 ENC_REQUIRED）、NAME_LEN u16、NAME、4 byte 境界まで 0 |

- 輸出名は ASCII の英小文字・数字・`_`・`-` の 1〜31 byte です。照合は大文字小文字を区別しません。
- 収まらなかったエントリがあれば、ヘッダの FLAGS bit0（MORE）を立てます。第 1 版に続きを取る方法はなく、サーバーは 1 応答に収まる数の輸出にとどめてください。
- 応答の遅延と待ち時間は spec_NET_L3P §5.5 に従います。
- 接続を許していない送信元（§10）には応答しません。

## 6. セッション内の操作

以下、表は BODY の配置です。応答の BODY が空の操作は「→ 空」と書きます。

### 6.1 fid の状態

fid は次の 3 状態を持ちます。

| 状態 | なり方 | できること |
|---|---|---|
| walked | ATTACH、WALK、CREATE（DIR） | WALK の起点、OPEN、CREATE の親、STAT、SETATTR（SIZE 以外）、REMOVE、RENAME の親、SYNC、STATFS、CLUNK |
| open（ファイル） | OPEN、CREATE（FILE） | READ、WRITE、STAT、SETATTR、SYNC、STATFS、CLUNK |
| open（ディレクトリ） | ディレクトリの OPEN | READDIR、STAT、SYNC、STATFS、CLUNK |

- 状態に合わない操作は `BAD_ARG`、知らない fid は `BAD_HANDLE`、使用中の番号を新しい fid に指定すると `EXISTS` です。
- open の fid が開いている対象は、FS2 §9 の BUSY 判定（開いている対象の削除・改名）に数えます。walked の fid は数えません。
- ファイルの読み書きの位置はクライアントが持ち、各 READ・WRITE が OFFSET を指定します。

### 6.2 ATTACH

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | FID（新しい fid） |
| 4 | 4 | UID（希望するアクセス UID。§10） |
| 8 | 2 | EXPORT_LEN |
| 10 | 2 | RSV |
| 12 | n | EXPORT（輸出名） |

Rattach: QID（16）、FLAGS u32（bit0 READ_ONLY）。知らない輸出は `NOT_FOUND`、許していない UID は `ACCESS` です。

### 6.3 WALK

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | FID（起点、walked） |
| 4 | 4 | NEWFID |
| 8 | 2 | NWNAME（0〜16） |
| 10 | 2 | RSV |
| 12 | … | NWNAME 個の `name` |

Rwalk:

| Offset | Size | Field |
|---|---|---|
| 0 | 2 | NQID（たどれた成分の数） |
| 2 | 2 | RSV |
| 4 | 4 | STATUS（全部たどれたら 0、途中で止まったらその成分のエラー） |
| 8 | … | NQID 個の `qid` |

- NWNAME = 0 は FID の複製です。NEWFID = FID は、全部たどれた場合だけ FID を置き換えます。
- 全成分をたどれたら NQID = NWNAME、STATUS = 0 で、NEWFID ができます。
- 1 つ目の成分で失敗したら `Rerror`、2 つ目以降で失敗したら NQID < NWNAME の Rwalk で STATUS にその成分のエラーを入れます。どちらの場合も NEWFID は作られず、FID は変わりません。
- 成分の意味は FS2 §11.1 と同じです。`..` は親で、輸出のルートの `..` はルート自身です（mount 境界をまたぐ `..` はクライアントの VFS が処理し、サーバーには送りません）。`.` と空の名前は `BAD_PATH` です。名前の検査は FS2 §6、照合は §8.2 に従います。
- 通過するディレクトリごとに探索権限（X）を検査します。ディレクトリでないものの先へは進めず `NOT_DIR` です。

### 6.4 OPEN と CREATE

Topen: FID（walked）u32、FLAGS u32。Ropen: QID（16）、IOUNIT u16、RSV u16。

- FLAGS は API §6.4 の bit の値をそのまま使います（READ `0x01`、WRITE `0x02`、TRUNC `0x10`、APPEND `0x20`、EXEC `0x40`）。CREATE・EXCL は OPEN では `BAD_ARG`（作成は CREATE で行う）。組み合わせの規則と必要な権限は API §6.4 と FS2 §8 に従います。
- ディレクトリは FLAGS = READ だけで開け、READDIR 用の open になります（R + X が必要）。それ以外の FLAGS では `IS_DIR` です。

Tcreate:

| Offset | Size | Field | 説明 |
|---|---|---|---|
| 0 | 4 | FID | 親ディレクトリ（walked） |
| 4 | 1 | KIND | 1 FILE、2 DIR |
| 5 | 1 | RSV | 0 |
| 6 | 2 | NAME_LEN | |
| 8 | 4 | FLAGS | FILE: OPEN と同じ規則（READ か WRITE を含む）。DIR: 0 |
| 12 | 4 | MODE | FS2 §8 の mode、または `0xFFFFFFFF`（`FND_MODE_DEFAULT`） |
| 16 | n | NAME | |

Rcreate: QID（16）、IOUNIT u16、RSV u16。

- 成功すると、FID は新しいものを指します。FILE なら FLAGS で開いた open の fid、DIR なら walked の fid です。
- 同名（照合キーが同じもの）がすでにあれば `EXISTS` です（常に EXCL と同じ）。親に W + X が必要です。

### 6.5 READ

Tread: FID u32、OFFSET u64、COUNT u16（1〜IOUNIT）、RSV u16。
Rread: COUNT u16、RSV u16、DATA。

- 返す長さが COUNT より短いのは EOF のときだけです（API §5 のファイルの規則）。OFFSET が末尾以降なら 0 byte です。

### 6.6 WRITE

Twrite: FID u32、OFFSET u64、COUNT u16（1〜IOUNIT）、RSV u16、DATA。
Rwrite: COUNT u16、RSV u16、END u64（書いたデータの直後の位置）。

- 全部書くかエラーです。エラーでも先頭の一部が書かれていることがあります（FS2 §9.1）。
- APPEND で開いた fid では OFFSET を無視して、実行時点の末尾に書きます。クライアントは END を新しい位置にします。
- 末尾より先への書き込みの穴は 0 で埋まります（FS2 §9）。

### 6.7 CLUNK・REMOVE・RENAME

Tclunk: FID u32 → 空。開いていたファイルの書き込みバッファは、応答の前に書き出します。

Tremove: FID u32、FLAGS u32（bit0 DIR）→ 空。

- DIR が 0 なら `FS_UNLINK`、1 なら `FS_RMDIR` の意味です。DIR = 0 でディレクトリなら `IS_DIR`、DIR = 1 でファイルなら `NOT_DIR`、空でないディレクトリは `NOT_EMPTY`、開いている対象は `BUSY`、輸出のルートは `ACCESS` です。
- 成否にかかわらず、FID は捨てられます（9P と同じ）。

Trename:

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | OLDDIR（walked） |
| 4 | 4 | NEWDIR（walked） |
| 8 | 2 | OLD_LEN |
| 10 | 2 | NEW_LEN |
| 12 | n | OLD_NAME、続けて NEW_NAME |

→ 空。意味は `FS_RENAME` と FS2 §9 の rename（同名への上書きは `EXISTS`、子孫への移動の禁止、開いている対象は `BUSY`）です。OLDDIR と NEWDIR が別の輸出なら `CROSS_DEVICE` です。2 つの名前の合計が 968 byte を超える組は送れず、クライアントは `NAME_TOO_LONG` を返します。

### 6.8 READDIR

Treaddir: FID u32（open のディレクトリ）、COOKIE u32、MAX u16（応答 BODY の上限、76〜980）、RSV u16。

Rreaddir:

| Offset | Size | Field | 説明 |
|---|---|---|---|
| 0 | 4 | NEXT | 次の Treaddir に渡す COOKIE |
| 4 | 2 | COUNT | エントリ数。0 なら終わり |
| 6 | 2 | RSV | 0 |
| 8 | … | エントリ | `stat`（64）、NAME_BYTES u16、NAME_CHARS u16、NAME、4 byte 境界まで 0 |

- COOKIE = 0 は先頭からです。それ以外の値の意味はサーバーが決めます。クライアントは受け取った NEXT だけを渡します。知らない COOKIE は `BAD_ARG` です。
- エントリは API §10.2 から rec_len を除いた形です。`.` と `..` は返しません。列挙の順序は規定しません。列挙中の変更の扱いは API §7 と同じで、同じエントリを 2 回返してはなりません。
- 名前が 900 byte を超えるエントリ、FS2 §6 の名前として不正なエントリ（ホストの不正な UTF-8 など）は返しません。
- MAX にエントリが 1 つも入らなければ `LIMIT` です。

### 6.9 STAT・SETATTR

Tstat: FID u32。Rstat: QID（16）、`stat`（64）。

Tsetattr:

| Offset | Size | Field | 説明 |
|---|---|---|---|
| 0 | 4 | FID | |
| 4 | 4 | MASK | bit0 MODE、bit1 UID、bit2 SIZE、bit3 MTIME。他は 0 |
| 8 | 4 | MODE | `FS_CHMOD` の意味 |
| 12 | 4 | UID | `FS_CHOWN` の意味 |
| 16 | 8 | SIZE | `ST_TRUNCATE` の意味。WRITE で開いた fid でなければ `ACCESS` |
| 24 | 8 | MTIME_SEC | i64 |
| 32 | 2 | MTIME_MS | 0〜999 |
| 34 | 2 | RSV | 0 |

→ 空。MASK が 0 なら `BAD_ARG` です。複数の bit を立てた場合の不可分性は保証しません（失敗した時点までの変更が残ることがある）。Foundation のクライアントは 1 bit ずつ送ります。MTIME はホストのツール向けで、Foundation の API からは使いません。

### 6.10 SYNC・STATFS

- Tsync: FID u32 → 空。open のファイルなら `ST_SYNC`、それ以外なら `FS_SYNC`（そのボリューム）の意味です。
- Tstatfs: FID u32 → `volinfo`（32）。SLOT は `0xFFFFFFFF`、FS_TYPE はサーバー側の backend（§8.1）です。

## 7. エラー

Rerror:

| Offset | Size | Field | 説明 |
|---|---|---|---|
| 0 | 4 | STATUS | 負の Foundation エラー番号 |
| 4 | 2 | MSG_LEN | 0〜128 |
| 6 | 2 | RSV | 0 |
| 8 | n | MSG | UTF-8 の説明（ログ・表示用。クライアントは解釈しない） |

- STATUS にはカーネル（[Foundation §4](spec_Foundation.md)）と API（API §3）のエラー番号をそのまま使います。主に使うもの: `BAD_ARG`、`BAD_HANDLE`、`ACCESS`、`LIMIT`、`NOT_FOUND`、`EXISTS`、`UNSUPPORTED`、`IO`、`READ_ONLY`、`BUSY`、`NOT_DIR`、`IS_DIR`、`NOT_EMPTY`、`BAD_PATH`、`NAME_TOO_LONG`、`NO_SPACE`、`CROSS_DEVICE`。
- 新しい番号 **-73 `FND_E_STALE`** を API §3 に加えます。意味は「ネットワークの相手がセッションまたはファイルを失った」です。クライアント側では L3P の結合が壊れた後の操作に、サーバー側では媒体の交換などで無効になった fid に使います。
- クライアントが知らない番号を受け取ったら `IO` として扱います。

## 8. サーバーの意味論

### 8.1 backend

サーバーは輸出ごとに 1 つの backend を持ちます。

| backend | FS_TYPE（volinfo） | 内容 |
|---|---|---|
| FS2 イメージ | 1 | `tools/fs2` と同じ意味論。更新ごとに FS2 §10 の検証を通してから保存する（FS2 §12.1 の 9P 公開と同じ方式） |
| ホストのディレクトリ | 4 | ホストのファイルを §8.2〜§8.4 の規則で見せる |

同じ FS2 イメージをゲストの IDC とサーバーで同時に書いてはなりません（FS2 §9）。

### 8.2 名前

- クライアントに見せる名前は、FS2 §6 の規則（UTF-8、禁止文字、1〜256 文字）を満たすものだけです。
- 照合は FS2 §6 の case folding で行います（Foundation の名前照合と揃えるため）。大文字小文字を区別するホストのファイルシステムでは、まず完全一致を探し、なければ照合キーが一致するものを探します。照合キーが同じ名前が 2 つ以上あるディレクトリでは、完全一致しないものの WALK を `NOT_FOUND`、READDIR では完全一致でない方を後のものから隠します（同じキーを 2 回見せない）。CREATE と RENAME は、照合キーが重なる名前を `EXISTS` にします。

### 8.3 属性

- ホストのディレクトリでは、UID は常に 0、mode はホストの所有者の権限を owner に、その他の権限を other に写します（group は無視）。SETATTR の MODE はこの逆写像で書き、UID は 0 以外を `UNSUPPORTED` にします。
- 時刻は modified = ホストの mtime、changed = ctime、created = 作成時刻（取れなければ mtime）です。
- シンボリックリンクは、輸出の内側を指すならその先として見せ、外側を指すものは見せません。特殊ファイル（デバイス・FIFO・ソケット）は見せません。
- QID の PATH はホストの inode 番号（と装置番号）から作り、GEN はファイルの作成時刻などから同一性が変わったときに変わるようにします。

### 8.4 BUSY

開いている対象の削除・改名を BUSY にする規則（FS2 §9）は、このサーバーのすべてのセッションの open の fid に適用します。ホストの他のプログラムによる変更は検出できません。

## 9. ホストのサーバー（Python）

第 1 版のサーバーは、telnet の TCP ブリッジ（`tools/cnet-bridge/cnet_bridge.py`、[spec_NET_bridge.md](spec_NET_bridge.md)）と同じ作りのホストプログラムとして作ります。

```
Foundation の netfs ──CFP／L3P／CNET── cfp_server.py ──▶ ホストのディレクトリ
                                                              └──▶ FS2 イメージ
```

| 項目 | 内容 |
|---|---|
| 置き場所 | `tools/cfp-server/cfp_server.py`、使い方は `tools/cfp-server/README.md`、テストは `tools/cfp-server/test_cfp_server.py` |
| 依存 | L3P と CNET の共有モジュール `tools/l3p/l3p.py`（spec_NET_L3P §10.2）。FS2 イメージの輸出は `tools/fs2/fs2.py` を import して使う。暗号化を使うときだけ X25519・ChaCha20-Poly1305 のライブラリ（`cryptography`）が要る |
| CNET への参加 | ブリッジと同じく、CNET の UDP カプセル化（spec_NET §8.3）を自分で話す 1 ノード。その処理と L3P は `tools/l3p/l3p.py` が受け持つ |
| ノードの役割 | 自分のノードへの PROBE に IN_USE を返し（spec_NET §5）、ソケット 4 のエコー（spec_NET §4.3）に応答する。`cnet ping .FF02` で到達を確かめられる |
| 受け付けるもの | DST_NET が 0 か自分の `--net` に一致し、DST_NODE が自分かブロードキャストで、ソケット 9 の L3P（PROTO `0x04`）のトランザクション |
| 並行性 | 1 つのスレッドで受信と処理を行い、要求を 1 つずつ実行する（結合ごとの順序は L3P が保証する、spec_NET_L3P §5.2）。L3P の再送とタイムアウトは実時間で計る |

### 9.1 起動オプション

セグメントへの接続のオプションはブリッジと同じ名前・意味にします。

| オプション | 意味 |
|---|---|
| `--bind HOST:PORT` | セグメントの UDP アドレス（既定 `0.0.0.0:32032`） |
| `--multicast GROUP:PORT` | セグメントのマルチキャストグループ |
| `--peer HOST:PORT` | ユニキャストの相手（複数可）。マルチキャストを使わないときは、届いたフレームの送信元も相手として覚える |
| `--segment-id N` | `udp.segment_id` と同じ値（既定 0） |
| `--net N` | セグメントのネットワーク番号（既定 0） |
| `--node HEX` | サーバーのノード番号（既定 `FF02`） |
| `--export NAME=PATH[,ro]` | 輸出（複数可）。PATH がディレクトリならホストのディレクトリ、ファイルなら FS2 イメージ。`ro` で読み取り専用 |
| `--allow ADDR` | 要求を受け付ける CNET アドレス（複数可、§10） |
| `--allow-any` | すべての送信元を受け付ける |
| `--max-sessions N` | 同時セッション（L3P の結合）数（既定 8） |
| `--require-encryption` | L3P の暗号化を必須にする（spec_NET_L3P §7.1） |
| `--key PATH` | サーバーの静的鍵（既定 `~/.cpt32/l3p/key`、なければ作る） |
| `--allow-key FINGERPRINT` | 受け付けるクライアントの静的鍵（複数可）。指定すると暗号化も必須になる |
| `--max-fids N` | セッションあたりの fid 数（既定 64） |
| `--verbose` | 要求と応答を 1 行ずつ表示する |

例（エミュレータの `config.yaml` の `peers` にサーバーのポートを入れておく。ブリッジと同時に使うときはポートを分ける）:

```powershell
python tools/cfp-server/cfp_server.py --bind 127.0.0.1:32041 --peer 127.0.0.1:32032 --net 1 --export share=C:\cpt32\share --export disk=build\foundation.fs2,ro
```

```
/> netfs exports .FF02
/> netfs mount .FF02 share s
/> ls /net/s
```

### 9.2 テスト

- `tools/cfp-server/test_cfp_server.py`: Python で書いた CFP クライアントから、UDP 越しにサーバーを試す。§14 のプロトコルの項目（損失・複製・順序入れ替えの注入を含む）をここで確かめる。
- `foundation/tests/cfp/`: testbench の Foundation と `cfp_server.py` を同じ UDP セグメントにつなぎ、`netfs` とシェルからの操作を確かめる（`foundation/tests/cnet/test_telnet.py` と同じ構成）。

## 10. 許可と安全

- 暗号化しない結合には認証がありません。CNET のセグメントに参加できる者は誰でもサーバーに要求を送れ、内容も見えます。マルチキャストで LAN に開いたセグメントにサーバーを置くときは、`--require-encryption` と `--allow-key` を使うか、輸出と許可を必要最小限にしてください。
- 暗号化した結合では、クライアントはサーバーの静的鍵を mount 名で覚えて確かめ、サーバーは `--allow-key` でクライアントを限れます（spec_NET_L3P §7.4）。
- サーバーは起動時の `--allow ADDR`（CNET アドレス、複数可）で要求を受け付ける送信元を限れます。`--allow-any` で全員を許します。何も指定しなければ、同じネットワークの全ノードを許します（ホストのディレクトリを輸出するときは、警告を表示します）。
- 輸出ごとに `ro`（読み取り専用。変更は `READ_ONLY`）を指定できます。
- ATTACH の UID は自己申告です。サーバーはそれを認証済みの UID とみなしてはなりません（FS2 §8）。第 1 版のサーバーは UID 0 だけを受け付け、それ以外は `ACCESS` にします。ホストのファイルへのアクセスは、サーバーを動かしているホストのユーザーの権限で行います。
- 輸出のルートの外（`..` や、外を指すリンク）には決して出てはなりません。
- 受信したメッセージはすべて信頼できない入力として、長さ・範囲・名前を検査します。

## 11. Foundation のクライアント（`netfs`）

### 11.1 構成

```
アプリ ─libfnd─▶ fs サービス ──(/net/<名前> のパス操作)──▶ netfs サービス ──/dev/cnet──▶ CNET
   │                                                          ▲
   └──────(開いたファイル・ディレクトリの CHANNEL)──────────────┘
```

- `netfs` はユーザーサービスで、`svc_register("netfs")` し、L3P（WAL `l3p`）の上で CFP を話します。
- mount 点は `/net/<名前>` です。`net` をルートの予約名（FS2 §11.1 の `dev/dd/os/tmp`）に加え、`/net` は仮想ディレクトリになります。
- fs は `/net/<名前>` の下のパス操作を netfs へ転送します。netfs は開いたファイル・ディレクトリの CHANNEL を自分の PORT で作り（デバイスサービスと同じ）、fs はそれをクライアントへ転送します。以後の読み書き・列挙は、fs を通らずクライアントから netfs へ直接届きます。
- fs は、ネットワークの応答待ち（最大 15 秒）の間もローカルのパス操作を止めてはなりません。転送の仕組み（fs の転送用スレッド、または netfs が fs から要求を引き取る方式）は実装計画で決めます（§13）。
- netfs は mount ごとに 1 つの L3P の結合（CFP のセッション）と、輸出のルートの fid を持ちます。L3P は WAL `l3p` を使います。

### 11.2 操作の対応

| Foundation の操作 | CFP |
|---|---|
| `FS_OPEN`（CREATE なし） | WALK → OPEN。CHANNEL を返す |
| `FS_OPEN`（CREATE） | 親へ WALK → 名前を WALK。NOT_FOUND なら CREATE（EXISTS が返ったら 1 回だけ WALK からやり直す）。見つかって EXCL なら `EXISTS`、なければ OPEN（TRUNC は OPEN の flags で） |
| `FS_STAT` | WALK → STAT → CLUNK |
| `FS_DIR_OPEN` | WALK → OPEN（READ）。CHANNEL を返す |
| `DIR_NEXT`／`DIR_REWIND` | READDIR（API §10.2 の rec_len を足して返す）／COOKIE を 0 に戻す |
| `FS_MKDIR` | 親へ WALK → CREATE（DIR）→ CLUNK |
| `FS_UNLINK`／`FS_RMDIR` | WALK → REMOVE（DIR = 0／1） |
| `FS_RENAME` | 両方の親へ WALK → RENAME → CLUNK ×2。mount が違えば fs が `CROSS_DEVICE` を返す |
| `FS_CHMOD`／`FS_CHOWN` | WALK → SETATTR（MODE／UID）→ CLUNK |
| `FS_SYNC` | WALK → SYNC → CLUNK |
| `FS_VOLUME_INFO` | STATFS。FS_TYPE は新しい値 **5 `FND_VOL_NET`**、SLOT は `0xFFFFFFFF` |
| `ST_READ`／`ST_WRITE` | 4096 byte までを IOUNIT ごとの READ／WRITE に分け、WINDOW の範囲で続けて送る。位置は netfs が持つ |
| `ST_SEEK`（END） | STAT で大きさを得る |
| `ST_STAT`／`ST_TRUNCATE`／`ST_SYNC` | STAT／SETATTR（SIZE）／SYNC |
| 最後のハンドルの close（`CLIENT_GONE`） | CLUNK |

- WALK は 16 成分ずつに分け、途中の fid は CLUNK します。
- ネットワークのエラーは、`TIMEOUT`（応答なし）、`STALE`（L3P の結合を失った）、`NO_MEDIA`（`/dev/cnet` が使えない、アドレス未取得）としてアプリに返します。
- L3P の結合が壊れたら（spec_NET_L3P §4.3）、そのセッションで開いていたファイル・ディレクトリの CHANNEL はすべて `STALE` を返すようになります。netfs は次のパス操作で新しい結合とセッションを作り、mount を続けます。

### 11.3 FS2 の VFS 規則との違い

- cwd を `/net/<名前>` の下に置いても、サーバー側の生存参照にはなりません（cwd のディレクトリを他のクライアントが削除・改名できる）。cwd はパスとして保持し、次の操作で解決し直します。
- BUSY の判定は、サーバーに open の fid がある対象だけです。
- 権限の検査はサーバーが行います。fs の VFS は `/net` と mount 点の探索権限だけを検査します。

### 11.4 シェルコマンド `netfs`

| 使い方 | 動作 |
|---|---|
| `netfs` | mount の一覧（名前、アドレス、輸出、状態） |
| `netfs exports [ADDR]` | 輸出の一覧。ADDR を省くとブロードキャストで探す |
| `netfs mount ADDR EXPORT NAME [-r] [-e]` | `/net/NAME` に mount する。`-r` は読み取り専用、`-e` は L3P の暗号化を使う（初回はサーバーの鍵の指紋を表示して確認する） |
| `netfs umount NAME` | mount を外す。開いているファイルがあれば `BUSY` |

アドレスの書き方は spec_NET §2 と同じです（`1.20`、`.FF02`）。

## 12. 既存仕様への変更

この仕様を実装するときに、合わせて次を変えます。

| 文書 | 変更 |
|---|---|
| [spec_NET_L3P.md](spec_NET_L3P.md) | CFP が使う既知のソケット 9 とノード `0xFF02`（L3P §11 に記載済み） |
| [spec_Foundation_API.md](spec_Foundation_API.md) | §3 に -73 `FND_E_STALE`、§10.3 に FS_TYPE 5 `FND_VOL_NET`、§2.3 に fs と netfs の間の操作の範囲（例 `0x0500–0x05FF`）を加える |
| [spec_FS2.md](spec_FS2.md) §11.1 | 予約名に `net` を加え、表に `/net/<名前>` = CFP を加える |
| [spec_Foundation_shell.md](spec_Foundation_shell.md) | コマンド表に `netfs` を加える |

## 13. 確認が必要な決定

| 論点 | 現在の案 | 他の案 |
|---|---|---|
| mount の場所 | `/net/<名前>`（予約名を 1 つ増やす） | 予約名を増やさず `/dd/<n>` の 4 以降を使う（IDC と番号空間が混ざる） |
| fs の待ち | fs がネットワーク待ちで止まらない仕組みを入れる | 第 1 版は fs が netfs を同期呼び出しし、最大 15 秒止まることを制限として受け入れる |
| サーバーの UID | UID 0 のみ | 輸出ごとに UID を写す表を持つ |
| 名前の上限 | 900 byte | L3P に複数パケットの応答（spec_NET_L3P §12）が入ったら 1024 byte まで扱う |
| Foundation 上のサーバー | 将来 | 第 1 版から `netfsd` を作り、Foundation 同士で共有する |

## 14. 実装時の検証項目

- 正常系: ATTACH、WALK（複製・部分失敗の STATUS）、OPEN、CREATE、READ（EOF の短い応答）、WRITE（APPEND の END）、READDIR（COOKIE で全件、重複なし）、STAT、SETATTR、REMOVE（DIR の組み合わせ）、RENAME（親をまたぐ、EXISTS、BUSY）、SYNC、STATFS、HELLO（版の不一致）。
- L3P の上で: 損失・複製・順序の入れ替えを注入しても、非冪等な操作（CREATE、REMOVE、RENAME、APPEND）が 1 回だけ実行されること。20% の損失で 1 MiB の転送が正しく終わること。結合が壊れたときに fid が片付き、クライアントが `STALE` を返し、新しい結合で再開できること。暗号化した結合で同じ項目が通ること（L3P 自体の検証は spec_NET_L3P §14）。
- 上限: 900／901 byte の名前、IOUNIT ちょうどと超過、MAX_FIDS、結合数、16／17 成分の WALK、合計 968／969 byte の RENAME。
- 不正入力: 短すぎる BODY、予約欄が 0 でない、不正な UTF-8 の名前、`.`・空の名前、輸出のルートの `..`、外を指すリンク。
- ホストのディレクトリ: 大文字小文字だけが違う名前の衝突、不正な UTF-8 のホストの名前を隠すこと、`ro` の輸出。
- Foundation: `/net/NAME` での `ls`・`cat`・`cp`・`mkdir`・`rm`・`mv`、`cd` と相対パス、セッション喪失時の `STALE`、ネットワーク待ち中にローカルのファイル操作が進むこと、`netfs exports` のブロードキャスト。
