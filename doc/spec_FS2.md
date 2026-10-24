# FS2 (File System 2) 仕様 v1

Revision 1.0、2026-10-22。Status: **ホストツール実装済み（§12 の 1、`tools/fs2/`）・ゲスト側は未実装**。
Foundation の標準ディスクファイルシステムを FS2 と命名する。従来の実装案にあった仮称 FDFS は本書で置き換える。
本書はディスク上の byte 列、操作の意味、仮想ファイルシステム (VFS) との境界を定める。
ホストツールは `tools/fs2/fs2.py`（§12 の 1）。ゲストドライバと Foundation の VFS はまだ実装されていない。

「〜しなければならない」「〜してはならない」は規範的な要求である。

## 1. 設計と DSFS との関係

[DSFS](spec_DSFS.md) の 512-byte block、big-endian、ブート署名、MSB-first の空きビットマップを引き継ぐ。
階層、長い名前、UID、権限、ミリ秒時刻、断片化した割り当てを追加するため、ディスク形式の互換性は持たない。
DSFS の version や magic を変更するだけでは変換できない。

| 項目 | FS2 v1 |
|---|---|
| ディレクトリ | 単一ルートの木構造、`.` / `..` は仮想的に解決 |
| 名前 | UTF-8、1〜256 Unicode scalar values、最大1024 bytes、大文字小文字を保存し比較は区別しない |
| 空き管理 | 1 block = 1 bit のビットマップ1枚、first-fit |
| ファイル配置 | FAT に似た連結割り当て。次 block 番号は各データ block の先頭に置く |
| 時刻 | 符号付き64-bit UNIX秒 + ミリ秒部 (0〜999) |
| 権限 | u32 UID、owner / other の read / write / execute、GID なし |
| VFS | FS2、DSFS、RAM FS、デバイス、OS 情報を同じパス・ハンドル操作で扱う |

ビットマップは「空きか使用中か」だけを持ち、ファイル内の順序は連結で表す。
独立した FAT 表やエクステント表は置かない。512 bytes のうち4 bytes が連結情報なので、データ容量は508 bytes/block となる。
空き総量が足りれば非連続の block を利用できる一方、ランダムアクセスには連結の走査またはメモリ上の索引が必要になる。

## 2. 整数・媒体・レイアウト

全 multi-byte 整数は big-endian。`u16/u32/u64` は符号なし、`i64` は2の補数である。
block は512 bytes、block 番号は [IDC](spec_IDC.md) の LBA と一致する。パーティション表は持たず raw image 全体を使う。
加算・乗算・切り上げはオーバーフローしない幅で検証してから利用しなければならない。

format 時に `total_blocks`、`boot_blocks`、`inode_count` を決め、以後は固定する。
`1 <= inode_count <= 0xFFFFFFFE` とし、実際には媒体容量にも制限される。

```text
boot_start    = 2
bitmap_start  = 2 + boot_blocks
bitmap_blocks = ceil(total_blocks / 4096)
inode_start   = bitmap_start + bitmap_blocks
inode_blocks  = ceil(inode_count / 4)       # inode は128 bytes
data_start    = inode_start + inode_blocks
```

| block 範囲 | 内容 |
|---|---|
| 0 | ブートセクタ |
| 1 | superblock |
| `[boot_start, bitmap_start)` | 予約ブート領域（0 blocks でもよい） |
| `[bitmap_start, inode_start)` | 空きビットマップ |
| `[inode_start, data_start)` | 固定長 inode 表 |
| `[data_start, total_blocks)` | ファイル・ディレクトリの連結データ block |

`data_start <= total_blocks <= device_capacity` を満たさなければならない。
媒体末尾の `total_blocks` 以降は使用しない。データ領域が0 blocks でも空のルートを持てる。
最小構成は `total_blocks=4, boot_blocks=0, inode_count=1` である。

## 3. ブートと superblock

block 0 の byte `0x000..0x1FD` は510-byte のブートコード、`0x1FE..0x1FF` は `55 AA`。
format はコードを0にして署名を書き、通常のファイル操作は block 0 と予約ブート領域を変更してはならない。

superblock は block 1 の512 bytesである。

| Offset | Size | Field | 値・意味 |
|---:|---:|---|---|
| `0x000` | 4 | magic | `46 53 32 00` (`FS2\0`) |
| `0x004` | 2 | version | `1` |
| `0x006` | 2 | flags | bit 0 = `DIRTY`、他は0 |
| `0x008` | 4 | total_blocks | ボリュームの block 数 |
| `0x00C` | 4 | boot_start | `2` |
| `0x010` | 4 | boot_blocks | 予約ブート領域の block 数 |
| `0x014` | 4 | boot_size | ブートイメージの有効 byte 数 |
| `0x018` | 4 | bitmap_start | §2 の式 |
| `0x01C` | 4 | bitmap_blocks | §2 の式 |
| `0x020` | 4 | inode_start | §2 の式 |
| `0x024` | 4 | inode_blocks | §2 の式 |
| `0x028` | 4 | inode_count | ルートを含む inode スロット数 |
| `0x02C` | 4 | data_start | §2 の式 |
| `0x030` | 4 | root_inode | `1` |
| `0x034` | 4 | casefold_version | `0x00100000` = Unicode 16.0.0 |
| `0x038` | 8 | created_sec | format 時刻、i64 UNIX秒 |
| `0x040` | 2 | created_ms | 0〜999 |
| `0x042` | 446 | reserved | すべて0 |

`0 <= boot_size <= boot_blocks * 512`。0 はブートイメージ未導入を表す。
予約領域の確保だけを行い `boot_size=0` としてもよい。
`install-boot` は予約領域内にイメージを置き、末尾を0埋めしてから `boot_size` を公開する。
予約領域の拡張は再formatが必要である。導入中の電源断に対するブートイメージの保全は保証しない。

Foundation の stage1 は magic/version、レイアウトと範囲、`DIRTY=0`、`0 < boot_size <= 2 MiB` を検査してから、
`boot_start` から `ceil(boot_size/512)` sectors を読む。
この領域は連結ヘッダを持たない生の連続領域で、[Foundation §11](spec_Foundation.md) の FNDBOOT を格納する。
最後の sector の padding は FNDBOOT の size に含めない。FS2 自体は Foundation 以外のブートイメージも格納できる。

## 4. ビットマップとデータ連結

- block `i` はビットマップ先頭から byte `i >> 3`、mask `0x80 >> (i & 7)`。1 = 使用中、0 = 空き。
- `[0, data_start)` の全 bit は1。`total_blocks` 以降のビットマップ領域全体の padding bit は0。
- 空き inode は inode の type で判定し、inode 用の別ビットマップは持たない。
- 割り当ては最小番号の空き block から順に探す。連続している必要はない。
- 使用する全 block の bit を1にして永続化してから、inode/連結で公開する。
  解放は inode/連結から参照を外して永続化した後に行う。

データ block の offset 0 に `next_block: u32`、offset 4〜511 に508 bytes の payload を置く。
`next_block=0` は終端、それ以外はデータ領域内の block 番号でなければならない。
ファイル内容は連結順の payload を連ねた先頭 `size_bytes` bytes。
`block_count = ceil(size_bytes / 508)` であり、連結の長さは必ずこの値に一致する。
最終 block の未使用 payload は0。長さ0では `first_block=0, block_count=0`。
循環、他 inode との共有、過不足のある連結、空き bit の block への参照は禁止する。
サイズは u64 だが、実容量の上限は `508 * (total_blocks - data_start)` bytes である。

## 5. inode

inode 番号は `1..inode_count`。番号 `n` の位置は `inode_start*512 + (n-1)*128`。
inode 0 は無効参照を表す。ルートは inode 1、親は自分自身である。

| Offset | Size | Field |
|---:|---:|---|
| `0x00` | 1 | type: 0 = 空き、1 = 通常ファイル、2 = ディレクトリ |
| `0x01` | 1 | flags = 0 |
| `0x02` | 2 | mode（§8） |
| `0x04` | 4 | uid |
| `0x08` | 4 | parent_inode |
| `0x0C` | 4 | first_block |
| `0x10` | 8 | size_bytes |
| `0x18` | 4 | block_count |
| `0x1C` | 4 | reserved = 0 |
| `0x20` | 8 | created_sec: i64 |
| `0x28` | 2 | created_ms |
| `0x2A` | 2 | reserved = 0 |
| `0x2C` | 8 | modified_sec: i64 |
| `0x34` | 2 | modified_ms |
| `0x36` | 2 | reserved = 0 |
| `0x38` | 8 | changed_sec: i64 |
| `0x40` | 2 | changed_ms |
| `0x42` | 62 | reserved = 0 |

空き inode は128 bytes 全0。inode 表末尾のスロット外 padding も全0。
ルート以外の使用中 inode は、ちょうど1個の使用中ディレクトリエントリから参照され、その親の inode 番号と `parent_inode` が一致する。
全使用中 inode はルートから到達可能でなければならない。ハードリンクとシンボリックリンクは持たない。
ディレクトリの親をたどると有限回でルートに着かなければならない。

## 6. ディレクトリと名前

ディレクトリは通常ファイルと同じデータ連結に **1040-byte 固定長エントリ**を並べる。
エントリは508-byte payload の境界をまたいでよい。block header はストリームに含めない。
`size_bytes` は1040の倍数で、空きスロットも含む。空のディレクトリは長さ0でも、全スロットが空きでもよい。

| Offset | Size | Field |
|---:|---:|---|
| `0x000` | 4 | inode（0 = 空き） |
| `0x004` | 2 | name_bytes（1〜1024） |
| `0x006` | 2 | name_chars（1〜256） |
| `0x008` | 1024 | UTF-8 name、有効 byte 以後は0 padding |
| `0x408` | 8 | reserved = 0 |

空きエントリは1040 bytes 全0。名前に終端 NUL は付けず `name_bytes` で区切る。
UTF-8 は最短形式だけを許し、surrogate、不正な継続 byte、U+10FFFF 超過を拒否する。
`name_chars` は復号した Unicode scalar values の個数で、表示上の文字幅や grapheme cluster 数ではない。
例えば日本語256文字を一律256 bytesに制限してはならない。

名前には `/`、U+0000〜U+001F、U+007F〜U+009F を含めてはならず、名前全体が `.` または `..` であってはならない。
空白、ドット、非ASCII文字はそれ以外では使用できる。末尾空白や末尾ドットを勝手に削除しない。
バックスラッシュは通常の名前文字であり、区切りではない。

比較キーは Unicode 16.0.0 の [CaseFolding.txt](https://www.unicode.org/Public/16.0.0/ucd/CaseFolding.txt) の
status **C と F** による full case folding で得る。S と T は使用しない。表にない code point はそのまま残す。
ロケール、ホストOS、ホスト言語の Unicode 版に依存させてはならない。
同一ディレクトリ内でキーが重複する名前は禁止する。別ディレクトリでは同じ名前を使える。
キーは名前より長くなってもよく、256文字/1024 bytes の制限は保存名だけに適用する。

v1 は Unicode 正規化を行わない。合成済み文字と結合文字列は別名として扱う。
`Readme` と `README`、`Straße` と `STRASSE` は同名、`é` と `e` + U+0301 は別名。
大文字小文字だけを変える rename は許可し、自分自身を重複判定から除く。

## 7. 時刻

各時刻は `(sec: i64, ms: u16)`。1970-01-01 00:00:00 UTC からの UNIX秒と、その秒の中のミリ秒部である。
`0 <= ms <= 999`。うるう秒とタイムゾーンは格納しない。
64-bit 全体を UNIXミリ秒として解釈したり、浮動小数点で保存したりしてはならない。
例えば epoch の1 ms前は `(-1,999)`、1.234秒後は `(1,234)`。

- created: 作成時に設定し、その後は変更しない。
- modified: ファイル内容変更、またはディレクトリのエントリ追加・削除・改名で更新する。
- changed: 内容・所有者・mode・親・名前の変更で更新する。rename で対象の modified は変えない。
- read とディレクトリ列挙では時刻を変更しない（atime は持たない）。

ミリ秒の**表現精度**と時計の**実測精度**は別である。秒までしか取れない時計は ms=0 とする。
時計を取得できない場合の新規時刻は `(0,0)` とし、これは epoch と区別しない。
時計の巻き戻しを許容し、created <= modified などの大小関係は mount 条件にしない。
Foundation の現在の `time_get` は RTC秒と tick を別々に返すだけなので、
サブ秒を提供する際は一貫した時計スナップショットを別途定義する。tick の剰余を無条件に RTC秒へ足してはならない。

## 8. UID と権限

UID は `0..0xFFFFFFFE`。0 = 管理者、`0xFFFFFFFF` は無効値。GID、ACL、setuid、setgid、sticky bit は持たない。
mode は owner と other の6 bitだけを用いる。

| bit | 意味 |
|---:|---|
| 0 / 1 / 2 | other の X / W / R |
| 3 / 4 / 5 | owner の X / W / R |
| 6〜15 | 0（未定義） |

owner=RWX, other=RX は `0x003D`。POSIX の mode 数値とは異なるので `0755` をそのまま保存しない。
アクセスUIDが所有者なら owner の3 bit、それ以外なら other の3 bitを使い、両者を合算しない。
UID 0 は mode の検査を省略できるが、read-only mount、媒体の書込禁止、構造上の制限は回避できない。

| 操作 | 必要な権限 |
|---|---|
| パス成分の探索 | 通過する各ディレクトリの X |
| list | 対象ディレクトリの R + X |
| read / write / execute | 対象ファイルの R / W / X |
| create / mkdir / unlink / rmdir | 親ディレクトリの W + X |
| rename / move | 移動元と先の親それぞれの W + X |
| chmod | 所有者または UID 0 |
| chown | UID 0 のみ |

削除に対象ファイルの W は要求しない。rmdir は空の場合のみで、ルートは削除・移動できない。
新規 inode の UID は認証済みアクセスUID。既定 mode はファイル `0x0034`（owner RW / other R）、
ディレクトリ `0x003D`。create/mkdir は明示 mode も受け付ける。umask や親からの権限継承はない。

VFS が権限検査を一元化し、ドライバに認証済みUIDを渡す。
要求本文の自己申告UID、PID、badge 値をそのままUIDとみなしてはならない。
Foundation の現行カーネルABIにはUID属性とその照会手段がないため、マルチユーザー運用の前に、
カーネル管理の資格情報または信頼されたセッション発行によるUIDとの対応を仕様化しなければならない。
単一ユーザーの起動時は認証済み全セッションをUID 0に固定できるが、それをユーザー分離の実装と呼んではならない。
デバイスの CHANNEL を直接渡す場合も、open 時に許可した操作を越える権限を転送してはならない。

## 9. 操作と整合性

最低限 format、mount、unmount、stat、list、create、mkdir、read、write、truncate、unlink、rmdir、rename、chmod、chown、sync、fsck を定義する。
列挙順は未規定。操作はボリューム単位で直列化し、同一媒体へのホスト・ゲスト同時書き込みは禁止する。

- format: メタデータを初期化し、root inode を type=2, uid=0, mode=0x003D, parent=1, size=0 とする。
  全時刻にformat時刻を設定し、DIRTY=0 とする。予約ブート領域は0埋めする。
- create/mkdir: 親と名前の一意性、空き inode、必要な block 数を検査する。空きディレクトリスロットを再利用し、なければ1040 bytes伸ばす。
  inode とデータを準備してから親エントリを公開する。
- write/truncate: offset と長さは u64。末尾より先への書き込みと伸長で生じる穴は0で埋め、スパース領域を作らない。
  新しい block の確保・初期化を先に行い、連結と inode の size/count を更新してから不要 block を解放する。
- unlink/rmdir: エントリを全0にしてから inode を全0にし、最後にデータ block を解放する。
  v1 では開いている対象への削除を BUSY とする（削除後も開いているファイルは導入しない）。
- rename: 同じボリューム内だけ。既存の別エントリへの上書きは EXISTS、自分自身への同じ保存名の指定は成功して何も変更しない。
  移動で親 inode を更新する。ディレクトリを自分自身または子孫へ移してはならない。
  v1 では開いている対象（その子孫のハンドル・cwd参照を含む）の改名・移動も BUSY。
  別mountへの移動は CROSS_DEVICE とし、自動copy/deleteはしない。
- read: EOF以降は0 bytes。read/write/seek はディレクトリに対して拒否し、専用の列挙操作を用いる。
- 容量不足は NO_SPACE、inode スロット不足は LIMIT。名前長超過は NAME_TOO_LONG、不正な名前は BAD_PATH。
  本書のエラー名は意味上の名称であり、数値とIPC構造は `spec_Foundation_API.md` で別途定める。

### 9.1 DIRTY と永続化

読み書き mount は他の変更に先立って DIRTY=1 を永続化し、mount 中は1のままにする。
sync は全変更を書き出すがDIRTYを消さない。unmount は参照中のハンドル/cwd/子mountがないことを確認し、
全データとメタデータを永続化し、最後に DIRTY=0 を永続化する。
順序の「後」はAPI呼び出し順だけでなく、下層の永続化完了を待つことを意味する。
下層がこれを提供しない場合、ホスト電源断に対する耐久性は保証できない。

512-byte sector 内の書き込みが不可分でも、1040-byteエントリ、親子inode、renameの更新全体は不可分ではない。
**ジャーナルはなく、異常終了で leak だけが残るとは保証しない。**
DIRTY=1 の媒体は通常mountを拒否し、オフラインfsckを必要とする。
I/O失敗後はDIRTYを維持して新規更新を止める。エラーを返した操作でも書込済みprefixが残りうる。

fsck は書込前に全構造を検査する。ビットマップ再構築や未到達inodeの解放は明示repairモードだけで行い、
修復前のイメージを保存する。重複名、複数親、循環、範囲外/共有block、途中で壊れた名前・連結など、
正しい内容を一意に決められない破損は自動修復せず報告する。
全検証を通過した場合だけ最後にDIRTY=0へ戻せる。ファイル内容の復元や全中断状態の自動回復は保証しない。

## 10. mount 検証

read-only を含む通常mountは次をすべて検証し、失敗すれば媒体を変更せず FORMAT（未対応版/flags/照合版は UNSUPPORTED）を返す。

1. ブート署名、magic、version、既知flags、casefold_version、DIRTY=0。
2. §2 の位置/個数の式、inode_countの範囲、ブートサイズ、デバイス容量、全計算の範囲。
3. 全reservedとpadding、metadataの使用中bit、ビットマップ末尾の0 bit。
4. inode type/mode/UID、空きinode全0、全ms部が0〜999、size/count/連結の一致。
5. rootがディレクトリかつparent=1。全使用中inodeの到達性、一意の親、親番号の一致、木構造。
6. 各ディレクトリのsizeが1040の倍数、全エントリの形式、UTF-8、文字数、同一親内の比較キーの一意性。
7. 全連結の範囲内終端、循環なし、所有blockの重複なし、参照blockのbit=1、最終payloadの0 padding。
8. 参照されない使用中データblockがないこと。DSFSと異なり、FS2の通常mountはleakも拒否する。

fsckの検査モードはDIRTYを許容して走査するが、安全でないポインタを追ってはならない。
連結走査回数はデータblock数、木の走査はinode数で制限する。検証途中の情報をクライアントへ公開しない。

### 10.1 Foundation の一時的な起動対策（KAN-38）

起動時の全走査を避ける暫定措置として、`foundation/boot/build.py` はfsサービスを
`FS2_FULL_MOUNT_CHECK=0` でビルドする。署名・superblock・位置/個数・DIRTY拒否、
bitmapとinodeの基本検査（rootのtype/parentを含む）は維持し、使用中inodeと空きblock数を初期化する。
全チェーン、ディレクトリエントリ、到達性/親参照、block共有・leakの検査は省略する。
このモードは完全な§10検証ではなく、DIRTY=0でも構造破損を見逃す。
媒体は事前にホストの `tools/fs2/fs2.py check` で検査する。
`build.py` の `FS2_FULL_MOUNT_CHECK` を1に戻して再ビルドすると完全検査を復元できる。
単独のドライバビルドの既定値、ホストツール、DSFSの検査は従来どおり。

## 11. 仮想ファイルシステム (VFS)

FS2 は永続化形式であり、VFS はFoundationのfsサービス内の共通インターフェースである。
仮想ノードを FS2 のデバイスinodeとして保存しない。

ホスト共有ディレクトリの接続では、[IDCの拡張予定](spec_IDC.md)としてIDC内で内容をFS2互換のsector列へ変換する。ゲストのVFSは通常のFS2 backendで `/dd/<n>/` にマウントし、ホスト共有専用のドライバを追加しない。これは未実装の設計方針で、変換後の媒体にも本書の構造・照合・mount検証を適用する。

```text
libfnd / アプリ
    → fsサービス: パス解決・mount表・資格情報・権限・ハンドル
        → FS2 driver → diskサービス → kernel blk_io → IDC
        → DSFS driver（初期はread-only）
        → RAM FS / dev FS / os FS
```

### 11.1 パスと名前空間

- パスはUTF-8、区切りは `/`。絶対パスは `/` から、相対パスはプロセスのcwdから解決する。
  IPCには絶対パスを送ってよいが、途中の `..` を単なる文字列操作で事前消去してはならない。
- 空のパスはBAD_PATH。連続 `/` は1個とみなす。`.` はその場、`..` は親、ルートの `..` はルート。
  各成分を順に探索して権限と型を検査するため、`/missing/../x` はNOT_FOUNDである。
  末尾 `/` はディレクトリを要求する。深さの固定上限はディスク形式には設けない。
- Foundation VFS の入力パス上限は終端NULを除き4096 bytes、1成分は256文字/1024 bytes。
  相対パスの絶対化後も上限を適用し、超過はNAME_TOO_LONG。深い木が媒体上で有効でも、長すぎる絶対パスはこのAPIでは扱えない。
  FoundationのIPCは1転送4096 bytesまでなので、操作ヘッダと2本のrenameパスを含む要求の分割・組立てはファイルAPI仕様で定める。切り詰めたパスで操作してはならない。
- 名前照合は仮想ディレクトリとmount名にも§6を適用する。DSFSはDSFSの有効な8.3名を照合し、作成制約は元形式に従う。
- mountをまたぐ `..` はVFS上の親を返す。例：`/dd/0/..` は `/dd` で、ディスクルート自身を返さない。
  最長の文字列prefixを機械的に選ぶのではなく、成分ごとにmount境界と権限を処理する。

| パス | backend |
|---|---|
| `/`（`/bin`, `/lib`, `/etc`, `/home` を含む） | 起動ディスクのFS2 |
| `/dev` | デバイスサービス／カーネルドライバの仮想ノード（con、null、fb0、sgu、sgc、trng、prng、uart0） |
| `/dd/<n>`（n=0〜3） | IDC slot n のFSルート |
| `/os` | プロセス・カーネル・サービス情報、read-only |
| `/tmp` | RAM FS |

ルートの `dev/dd/os/tmp` はcase非区別の予約名。VFSが下層の同名を隠し、ルート列挙で重複させず、同名の作成・削除・改名を拒否する。
FS2単体のホストツールでは予約しない。仮想rootとmountpointにもUID/modeを与え、同じ探索権限を検査する。
仮想ディレクトリの既定値はuid=0、mode=0x002D（owner/other RX）。RAM FSの `/tmp` はuid=0、mode=0x003F（全UID RWX）。
デバイスノードのUID/modeと許可操作は信頼された起動設定で指定し、設定にないノードは公開しない。
`/os` は全UIDに対してread-only。生sectorアクセスは `/dev` に公開せずカーネルの専用権限を使う。
起動ディスクは `/` と `/dd/<boot-slot>` で同じvolumeオブジェクトを共有し、二重の書込mountや別キャッシュを作らない。
disk サービスの先読みキャッシュは write-through とし、書込を試みる前に同じ drive の
重なる全窓を無効化する。IDENTIFY は該当 drive の全窓を無効化し、read/write のエラーは
全窓を無効化する。先読みだけが失敗した場合は要求された範囲の読み直しを許すが、
失敗した先読みの部分データをキャッシュとして公開してはならない。
親の見え方はVFSのmount位置で決まるため、ハンドル/cwdにはvolumeとinodeだけでなくmountの識別も保持する。

### 11.2 共通操作・ハンドル

backend の共通操作は `lookup/stat/readdir/create/mkdir/read/write/truncate/unlink/rmdir/rename/chmod/chown/sync`。
未対応操作はUNSUPPORTED、read-only backendへの変更はREAD_ONLY。backendの制約を黙って別操作へ置換しない。

VFSはopen時に資格情報と許可modeを確認し、ハンドルをクライアントセッションに結び付ける。
ハンドルの同一性は `(mount, volume, inode, media_generation)` とする。inodeを再利用しても古いハンドルを別ファイルへ付け替えない。
すでに許可したread/writeはchmod後もcloseまで利用できる。所有者/UIDが変化してもハンドル権限を自動昇格しない。
open中の削除/改名をBUSYにする規則は全mount別名を通して判定する。
cwdもディレクトリへの生存参照として扱い、親変更・削除・unmountに同じ規則を適用する。

媒体取り外しで当該世代のハンドルを無効化し、NO_MEDIAを返す。再挿入は新世代としてmountする。
DSFSにはUID/modeがないため、read-only mountで仮想uid=0、ファイルmode=0x0024（owner/other R）、root=0x002D（owner/other RX）を与える。
DSFSの秒時刻は64-bitへゼロ拡張しms=0とする。実行可否の追加方針はVFS側で明示し、媒体にない権限を書き込まない。

## 12. 実装順序と受け入れ条件

1. ホストのformat/mount/fsck/ls/put/get/mkdir/rm/mv/chmod/chown/install-boot（配置予定 `tools/fs2/`）。
2. ゲストのFS2 read-onlyドライバ。ホストと同一イメージ・同一結果を確認。
3. ゲストの更新、DIRTY、fsck連携、セクタ書込境界ごとの障害注入。
4. VFSとRAM/dev/os/DSFS backend、UID認証経路、libfnd、ディスク起動。

以下は将来の実装に対する必須テストであり、実行済みテストではない。

- 境界サイズ0/1/508/509 bytes、断片化媒体、inode不足とblock不足、末尾ゼロ、truncate/穴埋め。
- 256文字のASCII/日本語/4-byte scalar名、257文字拒否、不正UTF-8、casefold拡張、重複とcase-only rename。
- `(-1,999)` と64-bit秒の往復、ms=1000拒否、RTC未取得、時計の巻き戻し。
- owner/other/root全権限、親X/W、所有者変更、自己申告UID拒否、DSFS/RAM/dev/osでの同じ検査。
- 空木、多階層、親子循環、異なる親への移動、共有/循環block、不正size、padding/reserved/bitmap破損。
- 各書込sector完了直後に中断しDIRTYが残ること。通常mount拒否、fsck報告/修復限界、内容復元を保証しないこと。
- `.`/`..`、mount越え、予約名のcase別名、別volume rename、二重mount別名のBUSY、媒体交換と古いハンドル。
- FNDBOOTのsize/予約領域/2 MiB上限、DIRTY媒体の起動拒否、BIOSからの実機経路。

契約例：2880 sectors、boot_blocks=0、inode_count=256では、bitmap_start=2、bitmap_blocks=1、
inode_start=3、inode_blocks=64、data_start=67。rootを除き255 inodeを作成できる。
最初の600-byteファイルはデータ2 blocksと、rootの1040-byteエントリ用3 blocksを必要とする。
空きinodeがあってもこの5 blocksがなければ初回create+writeは完成しない。

v1の対象外：ジャーナル、操作全体の電源断atomic性、ACL/GID、リンク、スパース、圧縮、暗号化、拡張属性、atime、オンライン容量変更。

### 12.1 ホストの9P公開

`tools/fs2/fs2.py serve` はrawイメージをTCPの9P2000として公開する。
既定は `127.0.0.1:5640`、認証なし、UID 0。`--read-only` で更新を拒否する。
9P2000.u / 9P2000.Lは提供しない。ホスト向けのオフライン操作用であり、
ゲストや他のホストツールとの同時更新には対応しない。

version/attach/flush/walk/open/create/read/write/clunk/remove/stat/wstatを提供する。
msizeは256〜1048576 bytes、walkは16成分まで。ファイルサイズは媒体容量で制限する。
ディレクトリreadは完全な9P statレコード単位とし、途中のoffsetは拒否する。
wstatは同一親内の改名、owner/other権限、数値UID、サイズ、mtimeを設定できる。
GIDは0固定、group表示はotherを複製し、groupのchmod指定は保存しない。
atime設定と特殊mode、ORCLOSE/OEXCL/OEXECは未対応。削除済みfidはstaleとして拒否し、
inode番号再利用時は世代を変える。removeは失敗時にもそのfidをclunkする。

複数接続の要求は直列化する。更新要求ごとにコピーへ適用し、§10検証を通してから
既存ホストツールと同じ一時ファイル置換で保存する。操作失敗時はコピーを破棄する。
成功応答前に保存するが、電源断時の永続性や複数要求全体のatomic性は保証しない。
手順と制限の詳細は `tools/fs2/README.md` を参照。
