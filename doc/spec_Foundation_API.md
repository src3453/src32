# Foundation API 仕様 v1

Revision 0.1、2026-10-24。Status: **ドラフト（主要な設計判断は確定、一部実装）**。libfnd は起動、ストリーム、ファイル・cwd、ディレクトリ列挙、プロセス、コンソールの一部を実装し、fs／consoleサービスをIPCで呼びます。現在の提供範囲は [`foundation/include/foundation/fnd.h`](../foundation/include/foundation/fnd.h) と [sol API](../sol/foundation/README.md) を参照してください。本書は Foundation OS のサービス間プロトコルと、アプリ向けライブラリ libfnd の API を定めます。対象は、ストリーム（ファイル、コンソール、デバイス）、fs サービスのセッションとパス操作、ディレクトリの列挙、コンソールの入出力、起動ブロックと標準ストリーム、プロセスの起動、名前照合、API のエラーコードです。カーネル ABI は [spec_Foundation.md](spec_Foundation.md)、ディスク形式と VFS の意味は [spec_FS2.md](spec_FS2.md)、シェルは [spec_Foundation_shell.md](spec_Foundation_shell.md) を参照してください。本書と [実装案](foundation_os_plan.md) が異なる場合は本書が優先します。

「〜しなければならない (MUST)」「〜してはならない (MUST NOT)」「〜してよい (MAY)」は規範的な要求を表します。

## 1. 範囲

| 項目 | 内容 |
|---|---|
| プロトコル | [Foundation §8](spec_Foundation.md) の IPC（`fnd_msg`、`ipc_call`、ハンドル転送）の上に載る、サービスとクライアントの約束 |
| サービス | fs（`svc_lookup("fs")`）、console（`svc_lookup("console")`）、デバイスサービス |
| ライブラリ | libfnd（`foundation/lib/fnd`、C11、WAL `fnd` 1.3 または静的リンク） |
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
| `0x0400–0x04FF` | pipe サービスの操作（§9.5） |
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
| ストリーム | `FS_OPEN`、`DEV_OPEN`（ファイル、コンソール、デバイス）、`PIPE_OPEN`（pipe の両端） | §5。コンソールなら §8 も |
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

- `ST_WRITE` のデータは UTF-8 のテキストです。LF は改行（行頭へ戻って次の行）、CR は行頭へ戻る、BS は 1 文字戻る、TAB は次の 8 桁境界へ進む、として扱います。ESC から始まるシーケンスは §8.1.1 のサブセットを解釈し、その他の制御文字は表示しません。
- ASCII のほか、私用文字 **U+E000–U+E0FF** を PCG の字形番号 **0x00–0xFF** に直接対応させます。UTF-8 の3バイトで送り、字形 0x00、0x09、0x0A、0x1B も制御動作を起こさず1セルへ表示します。ANSI の色・反転・折返しは通常の文字と同じです。字形の形状は設定済み PCG フォントに従い、フォントや CLUT は変更しません。この範囲以外の非 ASCII 表示対象文字は、8×16表示（§8.1.2）の全角文字（下記）を除いて `?` です。UTF-8 の1文字は1回の `ST_WRITE` 内に収めてください。
- **全角文字（8×16表示のみ）**：JIS X 0208 の文字（U+00A2 など一部の記号、かな、記号、全角英数、漢字）と U+FFFD は、UTF-8 から UTF-16 の code unit（BMP）へ変換し、PCG mode 3 の2 byte文字として **2セル幅**で表示します（左セルが上位 byte、右セルが下位 byte で、前景属性 bit 7:6 = `01`／`10`。[VDP 仕様](spec_VDP.md) 2.1）。色・反転はANSIの現在値が2セルに同じく適用されます。補助平面の文字、JIS X 0208 以外の文字、40列・80列（8×8）表示では従来どおり `?` 1セルです。
  - 全角文字は行をまたぎません。最終列では、その列を空白にして次の行の先頭へ折り返してから表示します（スクロールも通常どおり）。
  - BS は全角文字を1文字として扱い、2セル戻ります。全角文字の片方のセルへ別の文字を書く、または消去（`ESC [ K`、`ESC [ J` など）が片方だけに及ぶと、残ったもう片方は空白（元の色）になります。カーソルの移動（`ESC [ C`／`D`／`H` など）、`CON_SET_CURSOR`、`CON_GET_SIZE` の列数は常にセル単位です。
- 画面の最下行を越えるとスクロールします。

#### 8.1.1 ANSI 出力サブセット

7-bit の `ESC`（0x1B）を使います。`CSI` は `ESC [`、`n` は十進数です。移動量・座標の省略または 0 は 1、消去モードと SGR の省略は 0 です。座標は **行、桁の順で 1 から**指定し、画面外は端へ丸めます。相対移動はスクロールしません。

| シーケンス | 動作 |
|---|---|
| `CSI n A/B/C/D` | 上／下／右／左へ n セル |
| `CSI n E/F` | n 行下／上の行頭へ |
| `CSI n G`、`CSI n d` | 桁／行を絶対指定（他方は保持） |
| `CSI row ; col H` または `f` | 行・桁を絶対指定。`CSI H` は左上 |
| `CSI n J` | 0: カーソルから画面末尾、1: 画面先頭からカーソル、2: 画面全体を消去 |
| `CSI n K` | 0: カーソルから行末、1: 行頭からカーソル、2: 行全体を消去 |
| `CSI s/u`、`ESC 7/8` | カーソル位置のみ保存／復元（共通の保存先） |
| `CSI ? 25 h/l` | カーソル表示／非表示 |
| `CSI p ; ... m` | SGR。以下の属性を左から適用 |

消去はカーソル位置を含み、現在の前景・背景色を持つ空白で埋めます。カーソル位置とスクロール位置は変えません。全消去後に左上へ移るには `ESC [ 2 J ESC [ H` を送ります（表記中の空白は送りません）。従来の `CON_CLEAR` は左上へ移動し、スクロールと解析途中のシーケンスをリセットしますが、色・カーソル可視性・保存位置は保持します。

SGR は `0`（属性初期化）、`1`／`22`（前景の高輝度化／解除）、`7`／`27`（反転／解除）、`30–37`／`40–47`（前景／背景の基本8色）、`90–97`／`100–107`（高輝度8色）、`39`／`49`（前景／背景の既定色）に対応します。初期状態と `SGR 0` は高輝度白の前景、黒の背景、高輝度化・反転なしです。`1` は基本8色の前景だけを高輝度にし、背景や既に高輝度の前景には作用しません。

色番号は ANSI 順（黒、赤、緑、黄、青、マゼンタ、シアン、白）を 0–7、高輝度版を 8–15 とします。`CON_SET_COLOR` も同じ番号を使い、高輝度化と反転を解除します。PCG の既存 RGB222 CLUT の index `[0,32,8,40,2,34,10,42,21,53,29,61,23,55,31,63]` へ対応づけ、CLUT 自体は書き換えません。他の描画機能が CLUT を変更した場合はその色になります。

解析状態・色・保存位置・カーソルは画面全体で共有し、別ストリームや別プロセスにも引き継がれます。シーケンスは複数の `ST_WRITE` に分割できます。各書き込みは到着順に連結して解析し、プロセス終了時には自動リセットしません。アプリは終了前に `SGR 0` と必要なカーソル表示復帰を送ってください。入力キー形式は変えず、端末応答も生成しません。

未対応の CSI／ESC は最終 byte まで消費して無視します。CSI は最大16引数、各値0–9999で、超過・不正な引数や中間 byte を持つ CSI は全体を無視します。未対応 SGR 属性は無視し、`38/48;5;n` と `38/48;2;r;g;b` は色情報をまとめて読み飛ばします（256色・RGB色は未対応）。OSC は BEL または `ESC \`、DCS／SOS／PM／APC は `ESC \` まで内容を捨て、保存用バッファは使いません。CAN／SUB は途中のシーケンスを中止し、CSI／ESC の途中の新しい ESC は解析をやり直します。途中のその他の C0 と DEL は無視します。8-bit CSI、下線・点滅、スクロール領域、代替画面、端末問合せは未対応です。

シーケンスの意味は [XTerm Control Sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html) を参照し、対応範囲・制限は本節を優先します。従来の LF の行頭復帰、TAB の空白書き込み、右端での即時折り返しは維持します。

#### 8.1.2 コンソール表示モード

表示モードは入力の `LINE`／`RAW` とは独立です。起動時は80列×30行、8×8字形です。すべてのコンソールストリームは同じ表示モードを共有します。

| `CON_SET_DISPLAY` の `a0` | 定数 | 列×行 | 字形 | 解像度 |
|---:|---|---|---|---|
| 0 | `FND_CON_DISPLAY_40` | 40×30 | 8×8 | 320×240 |
| 1 | `FND_CON_DISPLAY_80` | 80×30 | 8×8 | 640×240 |
| 3 | `FND_CON_DISPLAY_8X16` | 80×30 | 8×16 | 640×480 |

定義済み以外の値（2を含む）は `FND_E_BAD_ARG` とし、画面も状態も変更しません。成功時は同じモードの再指定でも画面を現在色で消去し、カーソル・スクロール・保存カーソルを左上へ戻し、解析途中のANSIシーケンスを中止します。色・高輝度化・反転・カーソル可視性・入力モード・入力キューは保持します。`CON_GET_SIZE` と `CON_SET_CURSOR` の範囲検査、ANSIの移動・消去・折返し、BS／TABとスクロールは選択後の列数とセル高に従います。mode 3のスクロールは1行16 pixelです。

8×16は専用VRAM `0x10021000`–`0x10021FFF` の既存字形を使います。Foundationは起動・表示切替（同じモードの再指定を含む）・消去・スクロールでフォント領域を読み書きしません。GUIエミュレータの `config.yaml` の `pcg.font_8x16`、またはlauncherの `PCG font 8x16` で事前に4096 byteのrawフォントを読み込んでください。未設定で領域が空なら8×16の文字は表示されません。既存8×8フォントの2バンクも保持します。UTF-8の表示範囲はASCIIと私用文字 U+E000–U+E0FF（§8.1）で、8×16表示ではさらにJIS X 0208の全角文字（2セル、§8.1）です。それ以外の表示対象文字は `?` です。コンソールのMMIO grantはVDPレジスタ・文字/属性面・PeC入力と、全角フォント領域（下記）だけで、この1 byte用フォント領域は含みません。

全角フォントは、起動時には置かず、コンソールが最初に8×16表示へ切り替えたときに一度だけ VRAM `0x10200000`–`0x103FFFFF` へ書き込みます（`WIDE_FONT_BASE` = `0x00200000`、code unit `u` の字形は `0x200000 + u * 32`）。字形は共通ROM（`assets/unifont-bmp.chr`）の JIS X 0208 の文字と U+FFFD で、Jiskan16 収録字形を優先し、残りは GNU Unifont を使い、ビルド時に `foundation/boot/wide_font.py` が生成しコンソールのイメージ（約240 KiB）へ含めます。配置は Unifont ROM と同じ並びなので、`GLYPH` コマンドと同じ字形です。RGB888ビットマップ（`/dev/fb0`、`0x200000`–`0x238400`）と重なるのは code unit 0x1C20 未満だけで、全角文字は使いません。他の範囲を `/dev/fb0` などで書き換えると、全角の字形が壊れます。

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
| `0x1009` | `CON_SET_DISPLAY` | `a0` = 表示モード（§8.1.2） | — |
| `0x100A` | `CON_GET_DISPLAY` | — | `a0` = 現在の表示モード（0／1／3） |

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

#### FIFO DAC（SGU v2.1）

同ヘッダーの `fnd_sgu_dac_config` / `fnd_sgu_dac_submit` / `fnd_sgu_dac_start` は
既存の `fnd_call` を使い、追加のWAL exportを必要としません。全opはWRITE権限が必要です。

| op | 値 | 契約 |
|---|---|---|
| `FND_SGU_DAC_CONFIG` | `0x1300` | a0=1（mono）/2（stereo）。両SGUを消音し、SGU0 ch0/1をFIFO DAC、48kHz、音量255に設定。monoはPAN=0xFF、stereoはch0=0xF0/ch1=0x0F。FIFO・保持値をリセットし、再生停止状態にする。旧SGUはUNSUPPORTED |
| `FND_SGU_DAC_SUBMIT` | `0x1301` | buf/len=符号付きBE PCM16、stereoはL,Rの順でインターリーブ。lenは4096以下かつフレームサイズの倍数。両FIFOの空きがあるフレームだけ投入し、応答a0=受理byte数、a1=投入後の最大FIFO残量（フレーム数）。全く入らなければWOULD_BLOCK。len=0は状態照会で常にOK |
| `FND_SGU_DAC_START` | `0x1302` | CONFIGした全chのPCM_CONTROLを同じカーネル呼び出し内で1にする |

CONFIG前のSUBMIT/STARTはBAD_ARGです。CONFIG/START/SUBMITはストリーム位置を変えません。
SUBMITは待機せず、左右同数のフレームを投入します。アプリは受理したbyte数だけ進め、未送信分を再試行します。
`fnd_sgu_dac_submit(h, pcm, bytes, &done, &queued)` は上のa0/a1を返し、エラー時done=0です。
FIFO DAC操作中に生のST_WRITEで関連レジスタを変更する使い方は対象外です。
START前に先行充填し、終了時は0フレームを送り、残量0を確認してcloseします。
close（強制終了を含む）は従来通り全chを消音・停止します。

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

#### libsgc（WAL `sgc` 1.0、`/lib/sgc.1.wal`）

`/dev/sgc` の上の高レベルライブラリです。ヘッダーは `<foundation/libsgc.h>`、実装は `foundation/lib/sgc`。`fnd` WAL を import する WAL で、`/lib/sgc.1.wal` として既定のディスクに入ります（利用側は `--import-wal` で `fnd` と `sgc` の両方を指定します）。静的状態は持たず、呼び出し側の `cpt_sgc` 文脈とコマンドバッファ（word 配列、`CPT_SGC_MAX_COMMAND`＝8 word 以上）を使います。

- `cpt_sgc_open` は `/dev/sgc` を WRITE で開きます。`cpt_sgc_display_init(first, count, rgb)` は `/dev/fb0` を開き、320×240・INDEX8 を確認して CLUT を書き、`cpt_sgc_close` まで開いたままにします（SGC の画素は fb0 ではなく SGC の描画面に描かれます）。
- 描画関数（`cpt_sgc_clear`／`rect`（x, y, 幅, 高さ）／`line`／`triangle`／`glyph`／`raw`）はコマンドをバッファへ積むだけです。色は 0〜63（範囲外は BAD_ARG）、直前と同じ色の SET_COLOR は省きます。座標は符号付き 16 bit に丸めます。面積のない矩形と一直線上の三角形（ハードウェアが FIFO_ERROR にする）は積まずに FND_OK を返します。
- `cpt_sgc_flush` は FIFO の空き（256 word）に入る分だけ投入し、待ちません。FIFO が満杯でも FND_OK で、残りは `cpt_sgc_pending` で分かります。描画関数はバッファが埋まると一度 flush を試し、なお入らなければコマンドを分割せず FND_E_WOULD_BLOCK を返します（呼び出し側は次のフレームで再試行）。`cpt_sgc_flush_wait` は 1 tick ずつ眠って空になるまで待ち、`CPT_SGC_WAIT_TICKS` 回進まなければ TIMEOUT です。
- `cpt_sgc_cancel`（未送信を捨てて FIFO clear）、`cpt_sgc_reset`、`cpt_sgc_set_output`、`cpt_sgc_status` は対応するデバイス操作です。cancel／reset と raw の後は現在色が不明になり、次の描画で SET_COLOR を送り直します。
- テストは `foundation/tests/libsgc`（ホスト側のモデルでコマンド列と FIFO 満杯・バッファ満杯を検証、Foundation 上で WAL を呼んで画素を確認）。`chip8` が利用者の例です。

### 9.4 PeC 乱数・UART `/dev/trng`・`/dev/prng`・`/dev/uart0`

`foundation/kernel/pec.c` のカーネルドライバです。fs は予約サービス
`dev.trng`、`dev.prng`、`dev.uart0` に `DEV_OPEN` を送り、CHANNEL を転送します。
`svc_register` でこれらの名前を登録すると EXISTS です。新しい syscall や WAL ABI は追加しません。

| ノード | mode（UID 0） | open | `ST_READ` | `ST_WRITE` |
|---|---|---|---|---|
| `/dev/trng` | `0x0024` | READ のみ | PeC TRNG。READY=0 ならブロック | ACCESS |
| `/dev/prng` | `0x0024` | READ のみ | PeC PRNG。常に待機せず読み出す | ACCESS |
| `/dev/uart0` | `0x0036` | READ／WRITE の一方以上 | RX_READY=0 ならブロック | DATA_RW へ全バイトを投入 |

- `DEV_OPEN` は READ／WRITE 以外の bit、または 0 を BAD_ARG にします。乱数デバイスに WRITE を指定すると ACCESS です。fs 経由の CREATE／EXCL／TRUNC／APPEND は §6.4 の規則に従います。
- 複数の open を許します。すべてのストリームは同じハードウェアの乱数列／UART 受信キューを消費します。dup・転送は元の権限を共有します。open／close はシード、有効化、UART 設定を変更しません。
- `ST_READ` は要求 `a0` と応答バッファ容量の小さい方まで返します。0 bytes は MMIO に触れず即時成功します。1 byte も取得できなければ、TRNG と UART は呼び出しスレッドだけを `PEC_READ` で待機させます。カーネルは 100 Hz のタイマーで再確認し、データ取得後に IPC 応答とともに起こします。WOULD_BLOCK や EOF は返しません。1 byte 以上あれば短い読み出しを許します。終了・kill 済みのスレッドへ遅延コピーしません。
- RNG は 32-bit 出力を上位 byte から返します。先頭 lane の読み出しで次の word を生成し、4 lanes を順に消費します。要求末尾が 4 の倍数でなければ残りの bytes は破棄します。TRNG は各 word の前で READY を確認します。PRNG は GFSR（初期シード `0x55555555`）であり暗号用途の保証はありません。独自の entropy pool は設けず、TRNG の待機条件はハードウェアの READY に従います。エミュレータでは有効な TRNG は通常 READY です（[PeC 仕様](spec_PeC.md)）。
- UART は変換・行編集をしないバイト列です。NUL もデータとして扱います。書き込みはカーネルの `debug_write` と同じく DATA_RW に投入し、全長を `a0` に返します。PeC エミュレータでは DATA_RW が送信キューに追加するため、TX_READY を待ちません。ホスト標準入出力への接続は UART の実装に従います。
- `ST_STAT` は DEVICE、上表の mode、UID 0、size 0 を返します。乱数デバイスの flags は READ_ONLY、UART は 0 です。`ST_SYNC` は成功する no-op、seek／truncate／その他の操作は UNSUPPORTED。権限のない read／write は ACCESS です。ユーザーバッファは IPC の検証後に `mm_copy_*` でコピーします。
- C アプリは既存の `fnd_file_open`、`fnd_read`、`fnd_write`、`fnd_close` を使えます。たとえば `fnd_file_open("/dev/prng", FND_OPEN_READ, FND_MODE_DEFAULT, &h)` で開いて任意長を読み出せます。

### 9.5 pipe サービス `pipe`

名前なしの pipe（POSIX の `pipe(2)`）のためのサービスです。`svc_lookup("pipe")` で得た CHANNEL は `PIPE_OPEN` だけを受け付けます。pipe の両端は、サービスの PORT から作った CHANNEL 1 本ずつで、badge で端を区別します。端のハンドルは `dup`・`XFER_HANDLE`・`proc_fork` で増やせます。端の最後のハンドルが閉じる（`CLIENT_GONE`）と、その端が閉じたことになります。

| op | 名前 | 要求 | 応答 |
|---|---|---|---|
| `0x0400` | `PIPE_OPEN` | `a0` = 0: 新しい pipe を作る。1: `a1` = pipe の id の書き端を付ける | `a0` = pipe の id、`a3` = 端のストリーム（`XFER_HANDLE`）。作る呼び出しは読み端を返す |

- 書き端は pipe ごとに 1 回だけ付けられます（付け済みなら `FND_E_NOT_FOUND`）。読み端と書き端の両方が閉じると pipe は消えます。
- 端のストリームは §5 のうち `ST_READ`（読み端）、`ST_WRITE`（書き端）、`ST_STAT`（`type` = 5 PIPE）、`ST_SYNC`（何もしない）を受け付けます。`ST_SEEK` と `ST_TRUNCATE` は `FND_E_UNSUPPORTED`、向きが違う端への読み書きは `FND_E_ACCESS` です。
- `ST_READ` はデータが届くまで待ち、届いた分（最大 `a0`）を返します。書き端がすべて閉じていてデータがなければ、長さ 0（EOF）を返します。
- `ST_WRITE`（4096 bytes まで）は、全体が入る空きができるまで待ち、まとめて書きます（原子的）。読み端がすべて閉じていれば `FND_E_PEER_GONE` です。待っている要求は端が閉じたときに答えます。
- 容量は 1 つの pipe につき 4096 bytes、同時に待てる要求は 32 件、pipe は 32 本です。超えると `FND_E_LIMIT` です。
- サービスはどのバックエンドでも起動します。ただし `fork` で端を共有できるのは VM だけです（`proc_fork` が VM 専用のため）。

## 10. 構造体

### 10.1 `fnd_stat`（64 bytes）

| Off | 名前 | 意味 |
|---:|---|---|
| 0 | `type` | 1 FILE、2 DIR、3 DEVICE、4 CONSOLE、5 PIPE |
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
    uint32_t           flags;     /* FND_EXEC_KEEP_ARGV0（1）: argv[0] を置き換えずそのまま渡す。他は予約。0 */
    const char *const *envp;      /* NULL 終端の "NAME=value"。NULL なら空の環境 */
} fnd_exec_t;

fnd_status fnd_proc_exec(const char *path, const fnd_exec_t *ex, fnd_handle *proc);
int32_t fnd_proc_fork(void);
fnd_status fnd_proc_wait(fnd_handle proc, int32_t *code);
fnd_status fnd_proc_kill(fnd_handle proc);
```

`fnd_proc_fork()` は WAL `fnd` 1.2 の追加 API です。カーネルの `proc_fork` を呼び、親に子の PROCESS ハンドル、子に 0、失敗時に負のエラーを返します。VM・単一スレッドなどの制約は [カーネル仕様 §9.6](spec_Foundation.md) を参照してください。ハンドルが指すファイル位置・fs セッション・cwd などのサービス側状態は親子で共有します。

環境変数は `WAL fnd 1.3` で加わりました。`fnd_envc()` と `fnd_env(i)` が、起動ブロック（[Foundation §9.3](spec_Foundation.md)、version 2）の環境を返します。子へは `fnd_exec_t.envp` で渡し、`fork` した子は環境をメモリごと引き継ぎます。環境と引数は合わせて起動ブロックの 4096 bytes に収まらなければならず、環境は 128 件までです。

`fnd_proc_exec` は次の手順で起動します。

1. `path` を `EXEC` で開き（X 権限の検査）、全体を `fnd_mem_alloc` の領域に読み込んで閉じる。大きさの上限は `FND_EXEC_IMAGE_MAX`（1.5 MiB。カーネルの `FND_SPAWN_IMAGE_MAX` も同じ）で、超えれば `FND_E_LIMIT`。
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
| 大きいイメージの起動 | `FND_EXEC_IMAGE_MAX` は 1.5 MiB。親のスロットより大きい子は、fs がカーネルへ直接渡す方式（将来）まで起動できない |
| WAL の一覧 | **決定（2026-10-22）**：`/lib/<image_id>.<abi_major>.wal` に置き、`fnd_proc_exec` が依存をたどって読み、`proc_spawn` の `libs` で渡す（§11.6、[Foundation §9.2・§9.5](spec_Foundation.md)） |
| sol のライブラリ | `sol/foundation/libfnd.sol` と `tools/solc/build_foundation.py` でC操作を利用可能。`examples/foundation-sol` は実FS2で5001 bytesのwrite/read/close、65件の列挙、rewind、ENDと資源解放、子の起動/待機を検証する |
| 64-bit 整数 | `tools/llvm-src32` での `int64_t` の対応を実装時に確認する。使えなければ、libfnd の 64-bit 引数を上位・下位の組に置き換える |
