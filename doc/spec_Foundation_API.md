# Foundation API 仕様 v1

Revision 0.1、2026-10-22。Status: **ドラフト（主要な設計判断は確定、一部実装）**。libfnd は起動、ストリーム、ファイル・cwd、ディレクトリ列挙、プロセス、コンソールの一部を実装し、fs／consoleサービスをIPCで呼びます。現在の提供範囲は [`foundation/include/foundation/fnd.h`](../foundation/include/foundation/fnd.h) と [sol API](../sol/foundation/README.md) を参照してください。本書は Foundation OS のサービス間プロトコルと、アプリ向けライブラリ libfnd の API を定めます。対象は、ストリーム（ファイル、コンソール、デバイス）、fs サービスのセッションとパス操作、ディレクトリの列挙、コンソールの入出力、起動ブロックと標準ストリーム、プロセスの起動、名前照合、API のエラーコードです。カーネル ABI は [spec_Foundation.md](spec_Foundation.md)、ディスク形式と VFS の意味は [spec_FS2.md](spec_FS2.md)、シェルは [spec_Foundation_shell.md](spec_Foundation_shell.md) を参照してください。本書と [実装案](foundation_os_plan.md) が異なる場合は本書が優先します。

「〜しなければならない (MUST)」「〜してはならない (MUST NOT)」「〜してよい (MAY)」は規範的な要求を表します。

## 1. 範囲

| 項目 | 内容 |
|---|---|
| プロトコル | [Foundation §8](spec_Foundation.md) の IPC（`fnd_msg`、`ipc_call`、ハンドル転送）の上に載る、サービスとクライアントの約束 |
| サービス | fs（`svc_lookup("fs")`）、console（`svc_lookup("console")`）、デバイスサービス |
| ライブラリ | libfnd（`foundation/lib/fnd`、C11、WAL `fnd` 1.1 または静的リンク） |
| ヘッダ | `foundation/include/foundation/{fnd.h,api.h}`。`api.h` は本書の番号と構造体、`fnd.h` は libfnd の関数 |

本書が定めないもの：

- FS2 の媒体上の構造と、各操作の意味（権限、BUSY、DIRTY など）は FS2 仕様に従います。本書は、それをどの IPC で呼ぶかと、エラーの数値を定めます。
- disk サービスと fs の間のプロトコル、init の起動設定の形式。
- sol 向けには [`sol/foundation/libfnd.sol`](../sol/foundation/libfnd.sol) から現在の C 実装を呼べます。[値・構造体・リンク方法と未対応操作](../sol/foundation/README.md) は別記します。仕様のみの API も含めて全操作が実装済みという意味ではありません。

## 2. 共通規約

### 2.1 メッセージ

- 要求は `fnd_msg` の `op` に操作コードを入れ、引数を `a0`〜`a3` と `buf`/`len` に入れます。使わない引数は 0 にしなければなりません。
- 応答の `op` には結果（`i32`）を入れます。0 以上は成功、負はエラーです（§3）。`ipc_call` 自体の結果（R1）は配送の成否で、操作の結果は応答の `op` にあります（[Foundation §8.3](spec_Foundation.md)）。
- 応答の値は `a0`〜`a3` に入れ、データは応答の `buf`/`len` で返します。
- 未知の `op` には `FND_E_UNSUPPORTED` を返します。そのオブジェクトの種類で使えない操作（ファイルに対するコンソール操作など）も同じです。
- 予約欄が 0 でない要求には `FND_E_BAD_ARG` を返します。

### 2.2 値の表し方

- 64-bit の値（ファイルの位置、大きさ、時刻の秒）は 2 つの語で渡し、上位語を先にします（`a1` = 上位、`a2` = 下位など）。構造体の中でも上位語を先に置きます。
- 符号付きの 64-bit 値（時刻の秒、SEEK の移動量）は 2 の補数です。
- 文字列は UTF-8 で、IPC のデータでは NUL で終端せず、`len` で長さを表します。NUL を含む文字列は `FND_E_BAD_PATH`（パス）または `FND_E_BAD_ARG`（その他）です。
- 1 回の転送のデータは 4096 bytes（`FND_IPC_MAX_DATA`）までです。

### 2.3 操作コードの範囲

| 範囲 | 用途 |
|---|---|
| `0x0000` | 予約 |
| `0x0001–0x00FF` | ストリーム操作（§5）。すべてのストリームに共通 |
| `0x0100–0x01FF` | fs セッション操作（§6） |
| `0x0200–0x02FF` | ディレクトリ操作（§7） |
| `0x0300–0x03FF` | デバイスサービスの操作（§9） |
| `0x1000–0x10FF` | コンソール操作（§8） |
| `0x1100–0x11FF` | VDPビットマップ／CLUT操作（§9.1） |
| `0x1200–0x12FF` | SGC 状態・出力GP・リセット操作（§9.3） |
| `0x8000–0xFFFEFFFF` | 各サービスの独自操作 |
| `0xFFFF0000–` | カーネルの通知（[Foundation §8.5](spec_Foundation.md)） |

## 3. エラーコード

カーネルの -1〜-63（[Foundation §4](spec_Foundation.md)）に加えて、次を定めます。FS2 仕様の意味上のエラー名は、右の列のとおりに対応させます。

| 値 | 名前 | FS2 の名前・意味 |
|---:|---|---|
| -64 | `FND_E_NOT_DIR` | パスの途中、またはディレクトリを要求した場所がディレクトリでない |
| -65 | `FND_E_IS_DIR` | ファイルを要求した操作の対象がディレクトリ |
| -66 | `FND_E_NOT_EMPTY` | 空でないディレクトリの rmdir |
| -67 | `FND_E_BAD_PATH` | BAD_PATH。空のパス、不正な名前、不正な UTF-8 |
| -68 | `FND_E_NAME_TOO_LONG` | NAME_TOO_LONG |
| -69 | `FND_E_NO_SPACE` | NO_SPACE |
| -70 | `FND_E_CROSS_DEVICE` | CROSS_DEVICE。別 mount への rename |
| -71 | `FND_E_INTERRUPTED` | Ctrl+C による入力待ちの中断（§8.4） |
| -72 | `FND_E_NEEDS_FSCK` | DIRTY の媒体の通常 mount を拒否した（FS2 §9.1） |

- -73〜-255 は予約です。
- FS2 の EXISTS、BUSY、LIMIT、ACCESS、READ_ONLY、FORMAT、UNSUPPORTED、NO_MEDIA、IO、NOT_FOUND は、カーネルの同名のコードを使います。
- 文字列への変換は `fnd_strerror`（§11.9）が行います。

## 4. オブジェクトとハンドル

### 4.1 種類

サービスがクライアントに渡すオブジェクトは、すべてカーネルの CHANNEL です。ユーザーサービスは自分の PORT から `chan_create` で CHANNEL を作り、badge でオブジェクトを区別します。カーネルドライバの CHANNEL は PORT を持たず、`ipc_call` 内で同期実行します（§9.1）。複製・転送・プロセス終了時の close は通常の CHANNEL と同じです。

| オブジェクト | 作る操作 | 使える操作 |
|---|---|---|
| fs セッション | `svc_lookup("fs")`、`FS_SESSION_NEW` | §6 |
| ストリーム | `FS_OPEN`、`DEV_OPEN`（ファイル、コンソール、デバイス） | §5。コンソールなら §8 も |
| ディレクトリ | `FS_DIR_OPEN` | §7 |

- 開いたファイル 1 つが 1 つの CHANNEL です。libfnd のファイルハンドルは、カーネルのハンドル番号そのものです（実装案 §4.4 の「別の名前空間」という案は採らない）。
- そのため、同時に開けるオブジェクトの数は、プロセスのハンドル表（63 個）から他の用途を引いた数です。

### 4.2 ケーパビリティとしての扱い

- CHANNEL を持つプロセスは、そのオブジェクトを開いたときに許可された操作だけを行えます。許可は開いた時点の資格情報と mode で決まり、後から変わりません（[FS2 §8・§11.2](spec_FS2.md)）。
- CHANNEL は IPC のハンドル転送や `proc_spawn` の `handles` で他のプロセスへ渡せます。渡した先でも許可は同じで、広がることはありません。
- `handle_dup` や `proc_spawn` で複製したハンドルは、同じオブジェクトを指します。ファイルの位置も共有します。
- サービスは、要求本文の PID や UID の自己申告を信用してはなりません。オブジェクトの識別には badge だけを使います。

### 4.3 後始末

- クライアントは `handle_close` でオブジェクトを閉じます。専用の CLOSE 操作はありません。
- オブジェクトを指す最後のハンドルが閉じられると（プロセスの終了を含む）、サービスに `FND_NOTIFY_CLIENT_GONE` が届きます。サービスはその badge のオブジェクトを解放しなければなりません。未完了の書き込みバッファがあれば、解放の前に書き出します。
- サービスが終了すると、クライアントの操作は `FND_E_PEER_GONE` になります。

### 4.4 badge の割り当て

- `svc_lookup` で得た CHANNEL の badge は、カーネルが割り当てます（bit 31 = 1）。fs ではこれが初期セッションです。
- サービスが `chan_create` で作る CHANNEL の badge は 1〜`0x7FFFFFFF` で、サービスが割り当てます。値の意味（オブジェクト表の添字と世代など）はサービスが決めます。解放した badge を、同じ PORT の別のオブジェクトにすぐ再利用してはなりません（古い CLIENT_GONE と区別できなくなるため）。

## 5. ストリームプロトコル

すべてのストリーム（ファイル、コンソール、デバイス）は、次の操作を受け付けなければなりません。種類として意味のない操作は `FND_E_UNSUPPORTED` を返します。

| op | 名前 | 要求 | 応答 |
|---|---|---|---|
| `0x0001` | `ST_READ` | `a0` = 最大 byte 数（1〜4096） | データ。`len` = 読んだ byte 数。0 は EOF |
| `0x0002` | `ST_WRITE` | データ（1〜4096 bytes） | `a0` = 書いた byte 数 |
| `0x0003` | `ST_SEEK` | `a0` = whence（0 SET、1 CUR、2 END）、`a1:a2` = 移動量（i64） | `a0:a1` = 新しい位置 |
| `0x0004` | `ST_STAT` | — | データ = `fnd_stat`（§10.1） |
| `0x0005` | `ST_TRUNCATE` | `a1:a2` = 新しい長さ | — |
| `0x0006` | `ST_SYNC` | — | — |

- `ST_READ` が返すのは、`a0` とクライアントの受信容量の小さい方までです。ファイルでは、EOF でなければ要求どおりの長さを返します（途中で短くなるのは EOF のときだけ）。デバイスとコンソールは、1 byte 以上が揃うまで応答を保留し、揃った分だけ返してよいです。
- `ST_WRITE` は、ファイルでは全部を書くかエラーです。エラーの場合、書き込み済みの先頭部分が残ることがあります（FS2 §9.1）。`APPEND` で開いたストリームは、毎回その時点の末尾に書きます。
- 読み取り権限のないストリームへの `ST_READ`、書き込み権限のないストリームへの `ST_WRITE` と `ST_TRUNCATE` は `FND_E_ACCESS` です。
- `ST_SEEK` で負の位置になる場合は `FND_E_BAD_ARG` です。末尾より先へは移動でき、そこへの書き込みで生じる穴は 0 で埋まります（FS2 §9）。コンソールとシーク不可のデバイスは `FND_E_UNSUPPORTED` です。
- コンソールに対する `ST_SYNC` は何もせず成功します。

## 6. fs セッションプロトコル

### 6.1 セッション

- セッションは fs のクライアントを表し、資格情報（UID）と cwd を持ちます。
- `svc_lookup("fs")` で得た CHANNEL は新しいセッションで、cwd は `/` です。
- v1 では、すべてのセッションの UID は 0 です（[FS2 §8](spec_FS2.md) の単一ユーザー運用）。これはユーザー分離ではありません。UID を認証する経路は、カーネル ABI の拡張と一緒に定めます。
- セッションの CHANNEL も §4.2 のとおり転送できますが、転送先と cwd を共有することになります。子プロセスには `FS_SESSION_NEW` で作った別のセッションを渡してください。

### 6.2 パスの渡し方

- パスは要求のデータ（`buf`/`len`）で渡します。長さは 1〜4096 bytes です。0 bytes は `FND_E_BAD_PATH`、4096 を超えるものは送れません（libfnd が `FND_E_NAME_TOO_LONG` を返す）。
- 相対パスは、fs がそのセッションの cwd から解決します。libfnd はパスを書き換えずに送ります。途中の `..` を文字列操作で消してはなりません（FS2 §11.1）。絶対化した後の長さにも 4096 bytes の上限を適用し、超えれば `FND_E_NAME_TOO_LONG` です。
- 実装案 §7 の「libfnd が絶対パスに変換して送る」は採りません。cwd の生存参照を持つ fs が解決する方が、改名や削除との整合を一か所で扱えるためです。

### 6.3 操作

| op | 名前 | 要求 | 応答 |
|---|---|---|---|
| `0x0100` | `FS_OPEN` | パス、`a0` = flags（§6.4）、`a1` = 作成時の mode | `a3` = ストリーム（`XFER_HANDLE`）、`a0` = 種類（§10.1） |
| `0x0101` | `FS_STAT` | パス | データ = `fnd_stat` |
| `0x0102` | `FS_DIR_OPEN` | パス | `a3` = ディレクトリ（`XFER_HANDLE`） |
| `0x0103` | `FS_MKDIR` | パス、`a1` = mode | — |
| `0x0104` | `FS_UNLINK` | パス | — |
| `0x0105` | `FS_RMDIR` | パス | — |
| `0x0106` | `FS_RENAME` | データ = 旧パスと新パスを連結したもの、`a0` = 旧パスの byte 数 | — |
| `0x0107` | `FS_CHMOD` | パス、`a0` = mode | — |
| `0x0108` | `FS_CHOWN` | パス、`a0` = UID | — |
| `0x0109` | `FS_SYNC` | パス（そのパスを含むボリューム） | — |
| `0x010A` | `FS_CWD_SET` | パス | — |
| `0x010B` | `FS_CWD_GET` | — | データ = cwd の絶対パス |
| `0x010C` | `FS_SESSION_NEW` | — | `a3` = 新しいセッション（`XFER_HANDLE`）。資格情報は同じ、cwd は複製 |
| `0x010D` | `FS_VOLUME_INFO` | パス | データ = `fnd_volinfo`（§10.3） |

- mode に `0xFFFFFFFF`（`FND_MODE_DEFAULT`）を渡すと FS2 の既定値（ファイル `0x0034`、ディレクトリ `0x003D`）になります。それ以外で bit 6〜15 が 0 でなければ `FND_E_BAD_ARG` です。
- 権限、BUSY、予約名、mount 境界、DSFS の読み取り専用などの意味は FS2 §8〜§11 に従います。
- `FS_RENAME` では、2 つのパスの合計が 4096 bytes 以下でなければなりません。超える組み合わせは v1 では `FND_E_NAME_TOO_LONG` です（パスを切り詰めて操作してはならない）。
- `FS_CWD_GET` の結果が受信容量に収まらない場合、fs は `FND_E_LIMIT` を返します（[Foundation §8.2](spec_Foundation.md)）。cwd は 4096 bytes 以下なので、4096 bytes の容量を用意すれば必ず収まります。
- `FS_CWD_SET` の対象はディレクトリで、探索権限（X）が必要です。

### 6.4 `FS_OPEN` の flags

| bit | 名前 | 意味 |
|---:|---|---|
| 0 | `READ` | 読み取り。R 権限が必要 |
| 1 | `WRITE` | 書き込み。W 権限が必要 |
| 2 | `CREATE` | なければ作る（親の W + X が必要） |
| 3 | `EXCL` | `CREATE` と一緒に使い、すでにあれば `FND_E_EXISTS` |
| 4 | `TRUNC` | 長さを 0 にする。`WRITE` が必要 |
| 5 | `APPEND` | 書き込みを常に末尾へ。`WRITE` が必要 |
| 6 | `EXEC` | 実行のための読み取り。R と X の両方が必要。`WRITE` とは併用できない |

- `READ`、`WRITE`、`EXEC` のどれもない場合と、矛盾する組み合わせは `FND_E_BAD_ARG` です。
- ディレクトリを `FS_OPEN` すると `FND_E_IS_DIR` です。列挙には `FS_DIR_OPEN` を使います。
- `/dev` のノードを開くと、fs はデバイスサービスにそのストリームを作らせ（§9）、受け取った CHANNEL をそのままクライアントへ転送します。ノードの UID、mode、許可する flags は起動設定で決まります（FS2 §11.1）。

## 7. ディレクトリの列挙

| op | 名前 | 要求 | 応答 |
|---|---|---|---|
| `0x0200` | `DIR_NEXT` | — | データ = エントリ（§10.2）を詰めたもの、`a0` = 件数。0 件なら終わり |
| `0x0201` | `DIR_REWIND` | — | — |

- 1 回の応答には、受信容量に収まるだけのエントリを入れます。1 件も入らない容量なら `FND_E_LIMIT` です。容量を 1096 bytes（エントリの最大）以上にすれば必ず 1 件は入ります。
- 列挙の順序は規定しません（FS2 §9）。並べ替えが必要なら、クライアントが行います。
- `.` と `..` は返しません。ルートでは、VFS の予約名（`dev`、`dd`、`os`、`tmp`）を 1 回だけ返します。
- 列挙中にディレクトリが変更された場合、変更されたエントリを返すかどうかは規定しません。ただし、同じエントリを 2 回返してはなりません。
- ディレクトリを開くには R + X が必要です（FS2 §8 の list）。

## 8. コンソール

console サービスは、画面（PCG/VDP）とキー入力を受け持ちます。コンソールのストリームは `/dev/con` を開くか、init が起動時に渡します。

### 8.1 出力

- `ST_WRITE` のデータは UTF-8 のテキストです。LF は改行（行頭へ戻って次の行）、CR は行頭へ戻る、BS は 1 文字戻る、TAB は次の 8 桁境界へ進む、として扱います。その他の制御文字は表示しません。
- 表示できない文字は代替の文字で表示します。どの文字を表示できるかは console の実装に任せます。
- 画面の最下行を越えるとスクロールします。

### 8.2 入力モード

コンソールのストリームは、入力モードを持ちます。モードはストリームのオブジェクトの状態で、そのストリームを指すすべてのハンドルで共有します。

| モード | `ST_READ` の動作 |
|---|---|
| 0 `LINE`（既定） | console が行を編集し、Enter で確定した行を LF 付きで返す。編集中の文字はエコーする。編集できるのは末尾への入力と Backspace だけ。1 行は 1023 bytes まで |
| 1 `RAW` | 入力された文字を、エコーせずに UTF-8 で返す。文字にならないキー（矢印など）は `ST_READ` に現れない |

- `LINE` で確定した行が `a0` より長い場合、残りは次の `ST_READ` で返します。
- Ctrl+D（行の先頭で押した場合）は `LINE` での EOF です。そのときの `ST_READ` は 0 bytes を返します。
- キー単位の入力（矢印などを含む）は、モードに関係なく `CON_READ_KEY` で読めます。
- シェルのように子プロセスへコンソールを渡すプログラムは、子を起動する前にモードを `LINE` に戻さなければなりません。

### 8.3 操作

| op | 名前 | 要求 | 応答 |
|---|---|---|---|
| `0x1000` | `CON_SET_MODE` | `a0` = モード | — |
| `0x1001` | `CON_READ_KEY` | `a0` = フラグ（1 = `NOWAIT`） | `a0` = キーコード、`a1` = 文字（Unicode scalar。なければ 0）、`a2` = 修飾キー。`NOWAIT` で渡せるキーがなければ（先に待っている要求がある場合を含む）待たずに `FND_END` |
| `0x1002` | `CON_BREAK_WAIT` | — | —（Ctrl+C が押されたら応答する） |
| `0x1003` | `CON_CLEAR` | — | — |
| `0x1004` | `CON_GET_SIZE` | — | `a0` = 桁数、`a1` = 行数 |
| `0x1005` | `CON_SET_CURSOR` | `a0` = 桁、`a1` = 行（0 から） | — |
| `0x1006` | `CON_GET_CURSOR` | — | `a0` = 桁、`a1` = 行 |
| `0x1007` | `CON_SET_COLOR` | `a0` = 前景、`a1` = 背景（0〜15） | — |
| `0x1008` | `CON_KEY_STATE` | — | `fnd_keyboard`（96 bytes）。READ 権限が必要 |

キーコード：

| 値 | キー | 値 | キー |
|---:|---|---:|---|
| 1 | Enter | 7 | ↓ |
| 2 | Backspace | 8 | Home |
| 3 | Delete | 9 | End |
| 4 | ← | 10 | Tab |
| 5 | → | 11 | Esc |
| 6 | ↑ | 0x100 | 文字（`a1` に文字） |

修飾キーは bit 0 Shift、bit 1 Ctrl、bit 2 Alt です。Ctrl と文字の組み合わせ（例：Ctrl+A）は、キーコード 0x100、`a1` = 文字（`a`）、Ctrl の bit で表します。

- 範囲外の座標と色は `FND_E_BAD_ARG` です。
- 入力の順序は、`ST_READ` と `CON_READ_KEY` で共通の 1 本の待ち行列です。両方を同時に待つクライアントがいる場合、先に届いた要求から順に満たします。

`<foundation/keyboard.h>` の `fnd_con_key_state` は物理 HID page 7 の usage 0〜255 を返します。
構造体は `uint32_t down[8], pressed[8], released[8]` の順で、usage `u` は word `u/32` の bit `u%32` です。
`down` は現在の長押し・同時押し状態、後の2配列は前回の取得以降に起きた遷移を表します。
キーリピートは pressed を追加しません。取得は押下キーの通常キューを消費せず、遷移だけをクリアします。
同じ取得間隔中の押下・解放は両方の bit が立ちます。回数・順序は保持しません。
状態と遷移は物理キーボード1台について共有されるため、複数アプリが遷移を別々には取得できません。
応答容量不足は LIMIT で、状態を消費しません。PeC FIFO の overflow 時には失われたイベントを復元できません。

### 8.4 中断（Ctrl+C）

- `CON_BREAK_WAIT` を待っているクライアントがいるとき、Ctrl+C は入力として扱いません。console は次のことをします。
  1. `CON_BREAK_WAIT` に応答する（成功）。
  2. そのコンソールで保留中の `ST_READ` と `CON_READ_KEY` を、すべて `FND_E_INTERRUPTED` で終わらせる。
  3. `LINE` モードの編集中の行を捨てる。
- 待っているクライアントがいなければ、Ctrl+C は通常のキー（キーコード 0x100、`a1` = `c`、Ctrl）として扱います。`LINE` モードでは、編集中の行を捨てます。
- `CON_BREAK_WAIT` を同時に待てるのは、コンソールあたり 1 件です。2 件目は `FND_E_BUSY` です。待っていたクライアントの CHANNEL が閉じられたら、待ちは消えます。
- これは [シェル仕様 §9.3](spec_Foundation_shell.md) の中断の仕組みです。シェルは専用のスレッドで `CON_BREAK_WAIT` を待ち、行編集中に `CON_READ_KEY` が `FND_E_INTERRUPTED` で戻ったら、入力中の行を捨てます。

## 9. デバイスサービス

- デバイスサービスは、`dev.<name>` の形のサービス名で登録します（例：`dev.uart0`）。console は `console` と `dev.con` の両方で登録します。
- fs は、起動設定の表に従って `/dev/<name>` を対応するサービスへ結び付けます。表にないノードは公開しません。

| op | 名前 | 要求 | 応答 |
|---|---|---|---|
| `0x0300` | `DEV_OPEN` | `a0` = flags（§6.4 の `READ`、`WRITE` だけ） | `a3` = ストリーム（`XFER_HANDLE`） |

- `DEV_OPEN` を呼べるのは fs だけにすべきですが、v1 のカーネル ABI には呼び出し元を確かめる手段がありません。そのため v1 では、デバイスサービスは誰からの `DEV_OPEN` も受け付けます。権限の検査は `/dev` の mode に頼ることになり、`svc_lookup("dev.<name>")` を直接使えば回避できます（§14）。

### 9.1 カーネル VDP ビットマップドライバ `/dev/fb0`

`/dev/fb0`（UID 0、mode `0x0036`）は `foundation/kernel/fb.c` のカーネルドライバです。fs は `svc_lookup("dev.fb0")` → `DEV_OPEN` で作った CHANNEL をクライアントに転送します。`dev.fb0` はカーネル予約名で、`svc_register` は EXISTS を返します。新しい SYSCALL、ユーザードライバプロセス、クライアントへの MMIO grant は不要です。通常の IPC と同じメモリ検証後、`mm_copy_*` でバッファにアクセスします。入力ハンドルの転送は BAD_ARG です。

初版はビットマップと CLUT だけを扱います。PCG、SGC、VPU、GP合成設定、DMA、mmap、VSync待ちは公開しません。

- `DEV_OPEN` は READ／WRITE の少なくとも一方を指定し、他のbitは BAD_ARG。同時に開けるストリームは1個で、2個目は BUSY。複製・転送は同じストリームと位置を共有します。fs経由の flags 処理は §6.4 のとおりです。
- 新規ストリームは INDEX8、位置0です。WRITE付きopenは Graphics表示・bitmap INDEX8・PCG overlay無効・display enable（共通 `0x80000000` に1）に切り替えます。READだけのopenは INDEX8領域とCLUTを読み、表示設定は変更しません。
- WRITE付きopenで、表示設定（enable、display mode、bitmap format、PCG overlay）、全256色のCLUT、INDEX8と重なるPCG文字・属性領域（VRAM `0x00000..0x02FFF`）を保存します。最後の参照を閉じると復元します。正常終了・fault・killによる破棄や、open後のハンドル確保失敗でも復元します。それ以外のビットマップ内容は保持します。
- 保存はスナップショットです。open中に他プロセスが行ったPCG文字・属性・CLUTへの変更はcloseで元に戻ります。初版はコンソールとの描画調停を行わないため、描画中のコンソール出力を避けてください。既存の特権MMIO利用者との競合も調停しません。
- `ST_READ`／`ST_WRITE` は選択形式の生画素byte列を操作します。libfndは4096 bytesずつ転送します。読み取りは末尾で短い応答、EOFは0 bytes。末尾を超える書き込みは、その要求全体を変更せず NO_SPACE。位置は成功したbyte数だけ進みます。
- `ST_SEEK` は SET／CUR／END、i64 offset。位置は `0..size` に限定し、範囲外は BAD_ARG。`ST_STAT` は DEVICE、mode `0x0036`、SEEKABLE、選択形式のsize（READだけなら READ_ONLY）。`ST_TRUNCATE` は WRITEなしなら ACCESS、WRITEありでも UNSUPPORTED。`ST_SYNC` は即時成功で、表示完了・VSyncを保証しません。

| op | 名前 | 要求 | 応答 |
|---|---|---|---|
| `0x1100` | `FB_GET_INFO` | — | データ = `fnd_fb_info`（24 bytes） |
| `0x1101` | `FB_SET_FORMAT` | `a0` = 0 INDEX8／1 RGB555／2 RGB888 | 位置を0へ戻す。画素は消去しない |
| `0x1102` | `FB_CLUT_READ` | `a0` = 先頭エントリ、`a1` = 件数 | RGB888列（件数×3 bytes） |
| `0x1103` | `FB_CLUT_WRITE` | `a0` = 先頭エントリ、`a1` = 件数、データ = RGB888列 | `a0` = 書いたbyte数 |

`fnd_fb_info` は big-endian u32 の `{width, height, stride, size, format, clut_entries}`。幅320、高さ240、CLUT256色です。strideは320／640／960、sizeは76800／153600／230400。画素byte順とVRAM配置は [VDP §2.2](spec_VDP.md#22-ビットマップgp) に従い、RGB555はbig-endian、RGB888はR,G,B順です。

CLUT操作はどの形式でも可能で、範囲は `first <= 256 && count <= 256-first`、件数0も許します。WRITEのデータ長は厳密一致（不一致・範囲外は BAD_ARG）。READの受信容量不足は LIMIT で、部分転送しません。先頭64色も操作でき、closeで復元します。

`FB_SET_FORMAT`／`FB_CLUT_WRITE`／`ST_WRITE` は WRITE、`FB_CLUT_READ`／`ST_READ` は READ が必要で、権限不足は ACCESS。未知の操作は UNSUPPORTED、未知の形式は BAD_ARG、応答容量不足は LIMIT。BMCの実VRAM容量に必要領域が収まらない形式は NO_MEDIA で形式・位置を変えません。容量未報告（0）も NO_MEDIA です。

C API は `<foundation/fb.h>` の `fnd_fb_get_info`、`fnd_fb_set_format`、`fnd_fb_seek`、`fnd_fb_clut_read`、`fnd_fb_clut_write`。既存 `fnd_call` を使うheader helperで、WAL ABIは追加しません。open/read/write/closeは既存libfnd APIです。実行例は [examples/foundation-fb](../examples/foundation-fb/README.md)。

### 9.2 SGU 2基と共有 PCMRAM `/dev/sgu`

`foundation/kernel/sgu.c` が提供するカーネルドライバです。UID 0、mode `0x0036`。
fs が予約サービス `dev.sgu` へ `DEV_OPEN` を送り、ストリーム CHANNEL を転送します。
`READ` / `WRITE` / 両方で開けます。全体で同時に1ストリームだけ利用でき、2回目の open は BUSY。
dup・ハンドル転送は権限、位置、寿命を共有します。利用者の MMIO grant は不要です。

| ストリーム offset | 対応する領域 |
|---|---|
| `0x0000..0x0FFF` | SGU 0（MMIO `0x80020000..0x80020FFF`） |
| `0x1000..0x1FFF` | SGU 1（MMIO `0x80021000..0x80021FFF`） |
| `0x2000..0x101FFF` | 共有 1 MiB PCMRAM（MMIO `0x18000000..0x180FFFFF`） |

各コアの波形 RAM は `core + channel*0x100`、制御レジスタは
`core + 0x800 + channel*0x20`（channel 0〜7）。レジスタの意味は [SGU 仕様](spec_SGU.md) に従います。
予約領域の read/write もハードウェアと同じく0／無視です。
例外として PHASE_RESET は **read で実レジスタを読まず0を返します**。位相をリセットするには write します。

`ST_READ`、`ST_WRITE`、`ST_SEEK`（SET/CUR/END）、`ST_STAT`、`ST_SYNC` をサポートします。
stream stat の size は `0x102000`、path stat は0。seek の有効位置は0〜size（終端を含む）で範囲外は BAD_ARG。
終端 read は0 bytes、read の終端越えは切り詰め、1 IPC 要求の write の終端越えは NO_SPACE で無変更です。
libfnd が分割する大きな write は、先行する要求分まで書ける場合があります。
権限不足は ACCESS、不明 op は UNSUPPORTED、SYNC は即時成功で音声再生完了待ちではありません。
ユーザーバッファは通常 IPC と同じ検証と `mm_copy_*` を通します。

WRITE で open したときと、そのストリームの最後のハンドルが閉じたときに、
両コア・全16チャンネルの VOLUME と PCM_CONTROL を0にします。
正常終了、fault、kill、ハンドル生成失敗でも解放・消音します。
波形、その他のレジスタ、PCMRAM の内容は保持し、以前の設定や再生位置は復元しません。
READ-only の open/close は音を変更しません。既存の特権 MMIO grant を持つプログラムとの排他調停は行いません。
`<foundation/sgu.h>` は offset 定数と `fnd_sgu_seek` を提供するヘッダーで、WAL ABI の追加はありません。

### 9.3 SGC 2D コマンド FIFO `/dev/sgc`

`foundation/kernel/sgc.c` が提供するカーネルドライバです。UID 0、mode `0x0036`。fs が予約サービス `dev.sgc`（`svc_register` は EXISTS）へ `DEV_OPEN` を送り、ストリーム CHANNEL を転送します。ハードウェアは [SGC 仕様](spec_SGC.md)。`READ`／`WRITE` のどちらか一方以上で開け、同時に開けるストリームは1個（2個目は BUSY）。dup・転送は権限と寿命を共有します。SGC の ID が `SGC1` でなければ NO_MEDIA です。

初版は **コマンド FIFO だけ**を公開します（線分・矩形・三角形・Unifont glyph・全面塗り・色指定）。スプライト（SAT／パターン）は扱わず、sprite count は 0 のままです。表示の有効化とCLUTは `/dev/fb0`（§9.1）か既存の仕組みで行い、SGC の色は 6-bit の CLUT index（先頭64色）です。SGC の描画面は GP に供給されるので、`/dev/fb0` のビットマップ（GP0）の上に重なります。

- WRITE 付き open でレジスタ（CONTROL、SAT_BASE、SPRITE_COUNT、OUTPUT_GP）を保存し、SGC を soft reset して有効化します（FIFO 空、描画面は透明、出力 GP 1、現在色0）。最後の参照が閉じる（正常終了・fault・kill・ハンドル確保失敗を含む）と、soft reset で FIFO と描画済み画素を捨て、保存したレジスタを戻します。READ だけの open は SGC を変更しません。
- `ST_WRITE` は SGC の FIFO word 列（big-endian、[SGC §7](spec_SGC.md)）を投入します。長さは4の倍数で、そうでなければ BAD_ARG。FIFO の空きに入る分だけ（word 単位）を投入し、`a0` に投入した byte 数を返します。要求より少ない場合があります（短い書き込み）。空きがなければ WOULD_BLOCK で、待ちません。コマンドの途中でも分割して投入できます。コマンドの内容はカーネルが検査せず、不正なコマンドは SGC の FIFO_ERROR になります。
- FIFO は VDP の1フレームにつき1回、完全なコマンドが消費されます（最大256 word で、満杯でも描画は進みます）。描画面はフレームをまたいで保持されるので、変化した部分だけを送れます。
- `ST_STAT` は DEVICE、mode `0x0036`、size 0（READ だけなら READ_ONLY）。`ST_SYNC` は即時成功で描画完了を保証しません。`ST_READ`、`ST_SEEK`、`ST_TRUNCATE` は UNSUPPORTED。

| op | 名前 | 要求 | 応答 |
|---|---|---|---|
| `0x1200` | `SGC_GET_STATUS` | `a0` = STATUS の sticky bit（0x1E の範囲）を W1C で消すマスク（0 なら消さない。非0は WRITE が必要） | データ = `fnd_sgc_status`（24 bytes）。消去前の値 |
| `0x1201` | `SGC_SET_OUTPUT` | `a0` = 出力 GP 0〜7（範囲外は BAD_ARG） | — |
| `0x1202` | `SGC_RESET` | — | soft reset して有効化。open 直後と同じ状態に戻す |
| `0x1203` | `SGC_FIFO_CLEAR` | — | FIFO と未完コマンドを捨てる。描画済み画素は残す |

`fnd_sgc_status` は big-endian u32 の `{id, control, status, fifo_used, fifo_free, output_gp}`。権限不足は ACCESS、応答容量不足は LIMIT、未知の操作は UNSUPPORTED。

C API は `<foundation/sgc.h>` の `fnd_sgc_get_status`、`fnd_sgc_set_output`、`fnd_sgc_reset`、`fnd_sgc_fifo_clear`、`fnd_sgc_submit`（空きの分だけ投入し、word 数を返す。空きが全くなければ WOULD_BLOCK）とコマンド定数。既存 `fnd_call` を使う header helper で、WAL ABI は追加しません。`fnd_write` は短い書き込みを IO にするので、`/dev/sgc` には `fnd_sgc_submit` を使ってください。

## 10. 構造体

### 10.1 `fnd_stat`（64 bytes）

| Off | 名前 | 意味 |
|---:|---|---|
| 0 | `type` | 1 FILE、2 DIR、3 DEVICE、4 CONSOLE |
| 4 | `mode` | FS2 §8 の mode |
| 8 | `uid` | 所有者 |
| 12 | `flags` | bit 0 `READ_ONLY`（mount か媒体が書き込み禁止）、bit 1 `SEEKABLE`。その他は 0 |
| 16 | `size_hi`／20 `size_lo` | 大きさ（byte） |
| 24 | `created_sec_hi`／28 `created_sec_lo` | 作成時刻（i64 UNIX 秒） |
| 32 | `created_ms` | 0〜999 |
| 36 | `modified_sec_hi`／40 `modified_sec_lo` | 更新時刻 |
| 44 | `modified_ms` | |
| 48 | `changed_sec_hi`／52 `changed_sec_lo` | 変更時刻 |
| 56 | `changed_ms` | |
| 60 | — | 予約。0 |

- デバイスとコンソールの大きさと時刻は原則0です。`fb0` のストリームstatは選択形式のビットマップbyte数を返します（パスstatの大きさは0）。
- `ST_STAT` と `FS_STAT` は同じ形式を返します。

### 10.2 ディレクトリのエントリ

| Off | 名前 | 意味 |
|---:|---|---|
| 0 | `rec_len` | このエントリの byte 数（4 の倍数） |
| 4 | `stat` | `fnd_stat`（64 bytes） |
| 68 | `name_bytes`（u16）／70 `name_chars`（u16） | 名前の byte 数と scalar value の数 |
| 72 | `name` | UTF-8 の名前（NUL なし）。4-byte 境界まで 0 で埋める |

名前は保存されている形のまま返します。最大のエントリは 72 + 1024 = 1096 bytes です。

### 10.3 `fnd_volinfo`（32 bytes）

| Off | 名前 | 意味 |
|---:|---|---|
| 0 | `fs_type` | 1 FS2、2 DSFS、3 RAM、4 仮想（dev、os） |
| 4 | `flags` | bit 0 `READ_ONLY` |
| 8 | `total_hi`／12 `total_lo` | データ領域の容量（byte） |
| 16 | `free_hi`／20 `free_lo` | 空き容量（byte） |
| 24 | `slot` | IDC slot（0〜3）。媒体がなければ `0xFFFFFFFF` |
| 28 | — | 予約。0 |

FS2 の容量は 508 bytes × データ block 数で数えます。

## 11. libfnd

### 11.1 型と規則

```c
typedef int32_t  fnd_status;   /* 0 = 成功、負 = エラー（§3） */
typedef uint32_t fnd_handle;   /* カーネルのハンドル番号。0 = なし */
```

- 関数は `fnd_status` を返し、結果は出力引数で返します。失敗したときは、出力引数を変更しません。
- 成功は 0 です。終わりに達したことを返す関数（`fnd_dir_next`、`fnd_con_read_line`）だけは、そのとき 1（`FND_END`）を返します。
- 文字列の引数は NUL で終わる UTF-8 です。
- libfnd の関数はスレッドから同時に呼んでよい（同じハンドルへの同時の呼び出しの順序は規定しない）。ただし `fnd_mem_*` は内部で排他します。
- 実装は `foundation/lib/fnd`、ABI の番号は `foundation/include/foundation/api.h` に置きます。

### 11.2 起動と終了

```c
fnd_status fnd_init(void);                /* 起動コードが main の前に呼ぶ */
uint32_t   fnd_argc(void);
const char *fnd_argv(uint32_t i);         /* 範囲外は NULL */
fnd_handle fnd_std_in(void);              /* 役割がなければ 0 */
fnd_handle fnd_std_out(void);
fnd_handle fnd_std_err(void);             /* なければ fnd_std_out() */
fnd_handle fnd_console(void);
fnd_handle fnd_fs(void);
void       fnd_exit(int32_t code);        /* proc_exit。戻らない */
```

- `fnd_init` は `proc_startup` で起動ブロック（[Foundation §9.3](spec_Foundation.md)）を読み、argv、cwd、役割表を取り出します。起動ブロックがない、または壊れている場合は、argc = 0、役割なしとして続けます。
- `FS` の役割がなければ、最初に fs を使うときに `svc_lookup("fs")` で新しいセッションを得ます。
- 起動ブロックの cwd は、fs のセッションの cwd に `FS_CWD_SET` で設定します。失敗した場合、cwd は `/` のままです。
- `main` から戻ると、その戻り値で `fnd_exit` します。

### 11.3 ストリームとファイル

```c
fnd_status fnd_file_open(const char *path, uint32_t flags, uint32_t mode, fnd_handle *out);
fnd_status fnd_read(fnd_handle h, void *buf, uint32_t len, uint32_t *got);
fnd_status fnd_write(fnd_handle h, const void *buf, uint32_t len, uint32_t *put);
fnd_status fnd_seek(fnd_handle h, int64_t off, uint32_t whence, uint64_t *pos);
fnd_status fnd_stream_stat(fnd_handle h, fnd_stat_t *st);
fnd_status fnd_truncate(fnd_handle h, uint64_t size);
fnd_status fnd_sync(fnd_handle h);
fnd_status fnd_close(fnd_handle h);       /* handle_close */
```

- `fnd_read` と `fnd_write` は 4096 bytes を超える要求を分割します。`fnd_read` は、EOF か、デバイスが短い応答を返したところで止めます。`fnd_write` は全部を書くか、エラーで止めます（書けた分は `*put` に入る。この場合だけ、失敗しても出力引数を設定する）。
- `fnd_file_open` の `mode` は `FND_MODE_DEFAULT` を使えます。

### 11.4 パスとディレクトリ

```c
fnd_status fnd_path_stat(const char *path, fnd_stat_t *st);
fnd_status fnd_file_delete(const char *path);
fnd_status fnd_file_rename(const char *old_path, const char *new_path);
fnd_status fnd_dir_make(const char *path, uint32_t mode);
fnd_status fnd_dir_remove(const char *path);
fnd_status fnd_path_chmod(const char *path, uint32_t mode);
fnd_status fnd_path_chown(const char *path, uint32_t uid);
fnd_status fnd_volume_info(const char *path, fnd_volinfo_t *vi);

fnd_status fnd_dir_open(const char *path, fnd_handle *out);
fnd_status fnd_dir_next(fnd_handle d, fnd_dirent_t *ent);   /* 終わりなら FND_END */
fnd_status fnd_dir_rewind(fnd_handle d);

fnd_status fnd_cwd_set(const char *path);
fnd_status fnd_cwd_get(char *buf, uint32_t cap, uint32_t *len);
```

- `fnd_dirent_t` は `fnd_stat_t stat`、`name_bytes`、`name_chars`、`char name[1025]`（NUL 終端）を持ちます。`fnd_dir_next` は `DIR_NEXT` の応答を内部にためて、1 件ずつ返します。
- C ABIでは長さ2欄はu32です。配置はstat=0、name_bytes=64、name_chars=68、name=72、全体1100 bytesです（§10.2のwire recordとは異なります）。`fnd_dir_rewind` は応答キャッシュも破棄し、`fnd_close` は確保したキャッシュを解放します。`FND_END` とエラー時には出力エントリを変更しません。これらと `fnd_file_rename` はWAL `fnd` 1.1の追加APIです。1.0の既存exportのシグネチャは変わりません。
- `fnd_cwd_get` の `cap` が足りなければ `FND_E_LIMIT` で、`*len` に必要な byte 数（NUL を含む）を入れます。

### 11.5 コンソール

```c
fnd_status fnd_con_write(const char *text);                 /* STDOUT へ書く */
fnd_status fnd_con_read_line(char *buf, uint32_t cap, uint32_t *len);
fnd_status fnd_con_getkey(fnd_key_t *key);                  /* CONSOLE の CON_READ_KEY */
fnd_status fnd_con_pollkey(fnd_key_t *key);                 /* 非 blocking、入力なしは FND_END */
fnd_status fnd_con_set_mode(uint32_t mode);
fnd_status fnd_con_clear(void);
fnd_status fnd_con_size(uint32_t *cols, uint32_t *rows);
fnd_status fnd_con_set_cursor(uint32_t col, uint32_t row);
fnd_status fnd_con_get_cursor(uint32_t *col, uint32_t *row);
fnd_status fnd_con_set_color(uint32_t fg, uint32_t bg);
fnd_status fnd_con_break_wait(void);
```

- `fnd_con_read_line` は STDIN から LF まで（LF は含めず NUL で終端）を読みます。STDIN がコンソールでもファイルでも同じです。`cap` を超える行は、`cap - 1` bytes で切り、残りを次の呼び出しに回して `FND_E_LIMIT` を返します。EOF で何も読めなければ `FND_END` を返します。LF のない最後の行は、その行を返し、次の呼び出しが `FND_END` になります。
- `fnd_con_write` は STDOUT、`fnd_con_getkey` 以下は CONSOLE の役割のハンドルを使います。
- `fnd_con_pollkey` は入力があれば `FND_OK` とキーを返し、なければ待たずに `FND_END` を返します。キー順序と修飾キー処理は `fnd_con_getkey` と共通です。CONSOLE に `CON_READ_KEY` を `a0` = `FND_CON_NOWAIT`（1）で送ります（§8.3）。

### 11.6 プロセス

```c
typedef struct {
    uint32_t           argc;
    const char *const *argv;      /* argv[0] を含む */
    const char        *cwd;       /* NULL なら現在の cwd */
    fnd_handle         std_in, std_out, std_err;   /* 0 なら自分のものを渡す */
    uint32_t           flags;     /* 予約。0 */
} fnd_exec_t;

fnd_status fnd_proc_exec(const char *path, const fnd_exec_t *ex, fnd_handle *proc);
fnd_status fnd_proc_wait(fnd_handle proc, int32_t *code);
fnd_status fnd_proc_kill(fnd_handle proc);
```

`fnd_proc_exec` は次の手順で起動します。

1. `path` を `EXEC` で開き（X 権限の検査）、全体を `fnd_mem_alloc` の領域に読み込んで閉じる。大きさの上限は `FND_EXEC_IMAGE_MAX`（1 MiB）で、超えれば `FND_E_LIMIT`。
2. `FS_SESSION_NEW` で子のための fs セッションを作る。
3. 起動ブロックを作る。cwd は `ex->cwd`、なければ `fnd_cwd_get` の値。役割表は 1 `STDIN`、2 `STDOUT`、3 `STDERR`、4 `FS`、5 `CONSOLE` で、`handles` にこの順に並べる（CONSOLE の役割がなければ 5 は省く）。
4. イメージが依存を持つ WAX v3 なら、依存する WAL を集める（[Foundation §9.5](spec_Foundation.md)）。依存表の各 `(library_id, abi_major)` について `/lib/<library_id>.<abi_major>.wal` を `EXEC` で開いて読み込み、その WAL の依存表も同じようにたどる。同じ ID と major は 1 回だけ読む。ファイルがなければ `FND_E_DEPENDENCY`、合計が WAX を含めて 8 個を超えれば `FND_E_LIMIT` で、起動しない。WAL の中身の検証と minor の確認はカーネルが行う。
5. `proc_spawn` を `priority = APP`、`caps = 0`、MMIO の付与なし、`name` = パスの最後の成分から `.wax` を除いた先頭 15 bytes、`libs` = 手順 4 の一覧（なければ 0）で呼ぶ。
6. 成否にかかわらず、イメージと WAL の領域と、手順 2 のセッションのハンドルを解放する（子には dup されている）。

- 起動ブロックが 4096 bytes を超える場合は `FND_E_LIMIT` で、起動しません。
- `fnd_proc_wait` は `wait(proc, FND_FOREVER, code)` を呼び、PROCESS ハンドルを閉じます。

### 11.7 名前照合

```c
fnd_status fnd_name_fold(const char *name, char *out, uint32_t cap, uint32_t *len);
int        fnd_name_equal(const char *a, const char *b);        /* 1 = 同名 */
int        fnd_name_match(const char *pattern, const char *name); /* 1 = 一致 */
```

- 照合は FS2 §6 と同じ（Unicode 16.0.0 の C と F による full case folding、正規化なし）です。libfnd は、fs と同じ版の変換表を持たなければなりません。
- `fnd_name_match` のパターンは `*`（0 文字以上）と `?`（1 scalar value）だけを特別に扱い、folding した後の文字列どうしで照合します。シェルのワイルドカード（[シェル仕様 §5.2](spec_Foundation_shell.md)）はこの関数を使います。
- 不正な UTF-8 には、`fnd_name_fold` は `FND_E_BAD_PATH`、他の 2 つは 0 を返します。

### 11.8 メモリ

```c
void *fnd_mem_alloc(uint32_t size);   /* 8-byte 境界。失敗は NULL */
void  fnd_mem_free(void *p);
```

ヒープは `mem_grow` で伸ばします。伸ばしたヒープをカーネルへ返す手段は v1 にはありません。

### 11.9 エラーの文字列

```c
const char *fnd_strerror(fnd_status code);
```

ASCII の英語の短い文を返します。カーネルのコードの文は [シェル仕様 §11.1](spec_Foundation_shell.md) の表のとおりです。API のコードは次のとおりです。表にないコードは `error` です。

| コード | 文 | コード | 文 |
|---|---|---|---|
| `FND_E_NOT_DIR` | `not a directory` | `FND_E_NO_SPACE` | `no space` |
| `FND_E_IS_DIR` | `is a directory` | `FND_E_CROSS_DEVICE` | `cross-device move` |
| `FND_E_NOT_EMPTY` | `directory not empty` | `FND_E_INTERRUPTED` | `interrupted` |
| `FND_E_BAD_PATH` | `bad path` | `FND_E_NEEDS_FSCK` | `needs fsck` |
| `FND_E_NAME_TOO_LONG` | `name too long` | | |

## 12. 起動時のハンドルの受け渡し

init は、各サービスの起動後にシェルなどのアプリを起動するとき、次の役割を渡します。

| 役割 | 値 |
|---|---|
| `STDIN`、`STDOUT`、`STDERR`、`CONSOLE` | console の 1 つのストリーム（同じオブジェクト） |
| `FS` | init の fs セッションから `FS_SESSION_NEW` で作ったセッション |

アプリから見れば、リダイレクトされていない STDIN と CONSOLE は同じオブジェクトです。`fnd_stat` の `type` が 4（CONSOLE）かどうかで、STDIN がコンソールかを判定できます。

## 13. 適合確認（実装時に追加するテスト）

以下は将来の実装に対する要求で、実行済みのテストではありません。

- **ストリーム**：ファイル、コンソール、`/dev/null` に同じ `ST_READ`／`ST_WRITE` が使える。4096 bytes を超える `fnd_read`／`fnd_write` が分割される。権限のない操作が `FND_E_ACCESS` になる。
- **ケーパビリティ**：読み取りで開いたストリームを子へ渡すと、子でも書き込みは `FND_E_ACCESS`。dup したハンドルで位置が共有される。最後のハンドルを閉じる（またはプロセスが終わる）と fs がファイルを解放し、BUSY が解ける。
- **セッション**：`FS_SESSION_NEW` のセッションの cwd を変えても、元のセッションの cwd は変わらない。相対パスが cwd から解決され、`/missing/../x` が `FND_E_NOT_FOUND` になる。
- **パス**：0 bytes、NUL 入り、4096 bytes を超える絶対化後のパス、合計が 4096 bytes を超える rename が、それぞれ §6.2〜§6.3 のエラーになる。
- **列挙**：エントリが応答の容量で分かれても、重複も欠落もない。ルートで予約名が 1 回だけ現れる。
- **コンソール**：`LINE` での行の分割返却、Ctrl+D の EOF、`RAW` でエコーされないこと、`CON_READ_KEY` の矢印キー。`CON_BREAK_WAIT` の待ちがあるときの Ctrl+C で保留中の読み取りが `FND_E_INTERRUPTED` になり、待ちがなければ通常のキーになる。2 件目の `CON_BREAK_WAIT` が `FND_E_BUSY` になる。
- **プロセス**：X 権限のないファイルの `fnd_proc_exec` が `FND_E_ACCESS` になる。起動ブロックの役割表が §11.6 のとおりになる。起動を繰り返しても、親のヒープ、ハンドル、fs のセッションが漏れない。
- **名前照合**：`fnd_name_equal("Straße", "STRASSE")` が 1、`é` と `e` + U+0301 が 0。`fnd_name_match("*.TXT", "readme.txt")` が 1。

## 14. 未決事項

| 論点 | 現在の案 |
|---|---|
| ファイルのハンドル | **決定（2026-10-22）**：開いたファイル 1 つ = 1 CHANNEL（§4） |
| コンソールの入力 | **決定（2026-10-22）**：`LINE` と `RAW` の 2 モード（§8.2） |
| UID の認証 | v1 は全セッション UID 0。カーネル ABI に資格情報を加えてから、`FS_SESSION_NEW` で UID を下げる操作などを定める |
| `DEV_OPEN` の呼び出し元の確認 | v1 では確認できない（§9）。カーネル ABI でサービスにケーパビリティ付きの CHANNEL を渡す方法を決めてから閉じる |
| 時刻の取得 | FS2 §7 の「一貫した時計のスナップショット」が未定義。libfnd の `fnd_time_now` はそれを待つ |
| 大きいイメージの起動 | `FND_EXEC_IMAGE_MAX` は 1 MiB。親のスロットより大きい子は、fs がカーネルへ直接渡す方式（将来）まで起動できない |
| WAL の一覧 | **決定（2026-10-22）**：`/lib/<image_id>.<abi_major>.wal` に置き、`fnd_proc_exec` が依存をたどって読み、`proc_spawn` の `libs` で渡す（§11.6、[Foundation §9.2・§9.5](spec_Foundation.md)） |
| sol のライブラリ | `sol/foundation/libfnd.sol` と `tools/solc/build_foundation.py` でC操作を利用可能。`examples/foundation-sol` は実FS2で5001 bytesのwrite/read/close、65件の列挙、rewind、ENDと資源解放、子の起動/待機を検証する |
| 64-bit 整数 | `tools/llvm-src32` での `int64_t` の対応を実装時に確認する。使えなければ、libfnd の 64-bit 引数を上位・下位の組に置き換える |
