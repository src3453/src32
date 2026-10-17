# DSFS (Dead Simple File System) 仕様

## 1. 概要

DSFSはフロッピーディスク用のシンプルなファイルシステムである。以下を特徴とする。

- 階層構造を持たない。ボリューム直下にファイルを並べるだけで、サブディレクトリやカレントディレクトリ、`.` / `..` は存在しない。
- 最大256ファイル。使用中スロットが256に達すると作成できない。
- ファイル名は8.3形式、大文字のみ。
- 所有者・権限・拡張属性などのユーザーデータを持たない。
- 時刻データは Unix Time（1970-01-01 00:00:00 UTC からの秒）で、秒単位である。
- 割り当て単位は512-byte sector 1個、すなわちブロック1個。空き管理は1ブロック1bitの単純なビットマップ1枚による。
- sector 0 はブートセクタとして予約する。

本仕様は raw ディスクイメージ上の byte列という外部契約を定める。IDC 仕様（`doc/spec_IDC.md`）の raw image としてそのまま扱え、DSFS の block 番号は IDC の LBA と一致する。実装言語や API 形式は定めない。

以降の「〜でなければならない」「〜してはならない」は仕様上の要求を表す。

## 2. ブロック構造

ブロックは512 bytes。全 multi-byte 整数は big-endian（`doc/spec_CPU.md` の byte order に合わせる）。レイアウトは次のとおりである。

| block 範囲 | 内容 |
|---|---|
| 0 | ブートセクタ |
| 1 | スーパーブロック |
| 2 .. `bitmap_start + bitmap_blocks - 1` | ビットマップ |
| `dir_start` .. `dir_start + dir_blocks - 1` | ディレクトリ（256エントリ） |
| `data_start` .. `total_blocks - 1` | データ領域 |

v1 では位置とサイズを次の式で定める。スーパーブロックにも同じ値を記録し、読み出し側は一致を検証する（§7）。

- `bitmap_start = 2`
- `bitmap_blocks = ceil(total_blocks / 4096)`
- `dir_start = bitmap_start + bitmap_blocks`
- `dir_blocks = 16`
- `data_start = dir_start + dir_blocks`
- `total_blocks >= data_start`（v1 では最低19 blocks）

データ領域が0 blocks のボリュームは許容する（0-byte ファイルのみ格納できる）。`total_blocks` がデバイス capacity より大きいイメージは無効である。capacity の方が大きい場合、`total_blocks` 以降の block は DSFS が使わない。

## 3. ブートセクタ

block 0 はブートセクタである。

| Offset | Size | 内容 |
|---:|---:|---|
| `0x000` | 510 | ブートコード領域。DSFS は内容を解釈しない。 |
| `0x1FE` | 2 | 署名 `0x55 0xAA` |

- ブートコード領域はファイル管理操作では変更しない。format 時は `0x00` で埋め、署名を書き込む。ブートローダは後からこの領域へ書き込んでもよいが、署名は維持しなければならない。
- 署名が無い block 0 のイメージは DSFS として無効である（§7）。

## 4. スーパーブロック

block 1 がスーパーブロックである。512 bytes、全 integer は big-endian。

| Offset | Size | Field | v1 の値 |
|---:|---:|---|---|
| `0x000` | 4 | magic | `"DSFS"` = `44 53 46 53` |
| `0x004` | 2 | version | `1` |
| `0x006` | 2 | flags | `0` |
| `0x008` | 4 | total_blocks | ボリューム全体の block 数 |
| `0x00C` | 4 | bitmap_start | `2` |
| `0x010` | 4 | bitmap_blocks | §2 の式による値 |
| `0x014` | 4 | dir_start | §2 の式による値 |
| `0x018` | 4 | dir_blocks | `16` |
| `0x01C` | 4 | data_start | §2 の式による値 |
| `0x020` | 4 | created | format 時刻（Unix Time 秒） |
| `0x024` .. `0x1FF` | — | reserved | `0` |

`created` はボリュームの format 時刻である。読み出し側は使用しない。

## 5. ビットマップ

ビットマップは1ブロック1bitで全 block を管理する。`bitmap_start` から始まる `bitmap_blocks` blocks に格納される。

- block 番号 `i` のビットは、ビットマップ先頭からの byte offset `i >> 3` の、byte 内 bit `0x80 >> (i & 7)` である（MSB-first。byte 0 の bit7 が block 0）。
- `1` = 使用中、`0` = 空き。
- 管理対象は `0 .. total_blocks - 1` である。format 時に `[0, data_start)`（ブートセクタ、スーパーブロック、ビットマップ自身、ディレクトリ）はすべて `1` にする。
- 最終 byte の、`i >= total_blocks` となる padding bit は `0` でなければならない。
- データ領域で、どの used entry からも主張されていない `1` bit（leak）は無効化の理由としない（§8 の更新順序により、異常終了で生じうるのはこの形だけである）。逆に、used entry が主張する領域の `0` bit は無効である（§7）。

## 6. ディレクトリ

ディレクトリは `dir_start` から始まる `dir_blocks = 16` blocks（8192 bytes）に、32-byte エントリを256個並べる。entry 番号 = byte offset / 32。エントリの順序は問わない。

### 6.1 エントリ形式

| Offset | Size | Field |
|---:|---:|---|
| `0x00` | 1 | status: `0` = 空き、`1` = 使用中。その他の値は無効。 |
| `0x01` | 11 | name: 8.3形式（§6.2） |
| `0x0C` | 4 | reserved = `0` |
| `0x10` | 4 | start_block（割り当て開始 block 番号） |
| `0x14` | 4 | size_bytes（ファイルサイズ） |
| `0x18` | 4 | created（Unix Time 秒） |
| `0x1C` | 4 | modified（Unix Time 秒） |

空きエントリは32 bytes すべて `0` でなければならない。status `0` で他の byte が非0のエントリは無効である。

### 6.2 ファイル名

name は byte 0..7 の8文字フィールドと byte 8..10 の3文字フィールドの並びで、ドット byte は持たない。表示時は `NAME.EXT`、拡張子が無い場合は `NAME` と表す。

- 許可文字は `A`–`Z`、`0`–`9`、`_`、`-` のみである。小文字、ドット、フィールド中間の空白は無効である。
- 各フィールドは非空白文字列とその後の空白 padding から成る。name フィールドは最低1文字でなければならず、ext フィールドは空白のみ（拡張子なし）でもよい。
- 使用中エントリどうしで、11-byte name が一致してはならない（大文字のみなので case 非区別の問題は生じない）。

### 6.3 サイズと割り当て

- `size_bytes = 0` のファイルは `start_block = 0` でなければならない。ブロックを1つも要求しない。
- `size_bytes > 0` のファイルが占有する block は `ceil(size_bytes / 512)` 個で、範囲 `[start_block, start_block + ceil(size_bytes / 512))` はデータ領域 `[data_start, total_blocks)` の内側でなければならず、他の used entry の占有範囲と重なってはならない。
- 最終 block の `size_bytes` より後の byte は、書き込み時に `0` で埋めなければならない。
- `created` は作成時に設定する。`modified` は内容の書き込みごとに更新する。rename は変更しない。どちらも0を含む任意の u32 値であり、時刻としての妥当性は検証しない。

## 7. マウント検証

読み出し側は次のいずれかでも満たさないイメージを DSFS として扱ってはならない。

1. block 0 の署名が `0x55 0xAA` でない。
2. スーパーブロックの magic、version、flags が §4 と一致しない。
3. スーパーブロックの `bitmap_start` / `bitmap_blocks` / `dir_start` / `dir_blocks` / `data_start` が §2 の式と一致しない。`total_blocks < data_start`、または `total_blocks` がデバイス capacity を超える。
4. スーパーブロックまたはエントリの reserved が0でない。
5. エントリで status が0/1以外、または status 0 なのにエントリ全体が全0でない。
6. status 1 のエントリで、name が §6.2 に反する、または使用中エントリ間で name が重複する。
7. status 1 のエントリで、`size_bytes = 0` なのに `start_block != 0`、または占有範囲がデータ領域外に出る。
8. used entry の占有範囲同士が重なる。
9. used entry が占有する block のビットが0である。
10. ブロック `0 .. data_start - 1`（メタデータ領域）のビットが0である。
11. ビットマップの padding bit が1である。

主張されない1 bit（leak、§5）はエラーではない。

## 8. 操作の規則

実装は最低限、format、mount（§7 の検証を含む）、list、create、read、write、delete、rename を持つ。

- **format:** ブートセクタ（領域0埋め + 署名）、スーパーブロック、ビットマップ、ディレクトリ（全0）を書き、`created` に現在時刻を入れる。データ領域の初期内容は不定であり、format は変更してよい。`total_blocks` が §2 の最小値を下回るイメージでは format できない。
- **割り当て:** データ領域の空きビットを先頭から探す first-fit で、連続する `ceil(size_bytes / 512)` blocks を割り当てる。ファイルは常に連続領域1つに置かれる。空き領域が総量では足りなくても、連続した空きが無ければ割り当てに失敗する（断片化による失敗）。
- **create:** 空きスロットを探し、name の一意性を確認してからエントリを作る。スロットが無い（256ファイル使用中）場合は作成できない。0-byte ファイルはブロックを要求しない。
- **write（新規作成時の内容書き込み）:** 1) 新領域のビットを1にする、2) データを書き、最終 block の残りを0埋めする、3) エントリを更新する（status/name/start/size/modified）。
- **write（既存ファイルの差し替え）:** 新しい占有領域を決める（現占有領域のままでもよい）。1) 新領域のすべてのビットを1にする、2) データを書き、最終 block の残りを0埋めする、3) エントリの start/size/modified を更新する、4) 旧領域のうち新領域に含まれない block のビットを0にする。占有領域が変わらない書き込みでは領域のやり直しは不要である。
- **delete:** 1) エントリを全0にする、2) その領域のビットを0にする。
- **rename:** name の一意性を確認のうえ name のみを書き換える。他のフィールドは変更しない。
- **read:** エントリの `[0, size_bytes)` を読む。

更新順序の不変条件は次の2つである。

- エントリが主張する領域のビットは、常に1である。
- ビットの解放（0化）はエントリの解放より後、ビットの占有（1化）はエントリの占有より前に行う。

これにより、sector 単位の書き込みが壊れないことを前提に、書き込み途中の異常終了で残るのは「どの used entry からも主張されない1 bit（leak）」だけに限られ、mount 不能な状態（§7-8/§7-9）は発生しない。ファイル内容自体の破壊は防げない（ジャーニャルは無い）。

## 9. 制限と非対応事項

- 階層構造、ディレクトリ、ファイル名の大文字小文字区別（小文字は無効）。
- 所有者・権限・ACL・拡張属性・ボリュームラベルなどのユーザーデータ。
- 削除済みエントリの保持（undelete）、fsck、ジャーニャル、スーパーブロック/ビットマップの冗長コピー。破損時の修復手段は format である。
- アクセス時刻、タイムゾーン、秒未満の時刻。
- 硬リンク、シンボリックリンク、スパースファイル。
- 256超過のファイル数、8.3を超えるファイル名。
- ファイル単位の断片化。連続割り当てのため、空き領域が断片化すると総空き量が十分でも作成/書き込みに失敗しうる。

## 10. 契約例

1.44 MiB フロッピー（2880 sectors）を format した場合:

- `total_blocks = 2880`、`bitmap_blocks = 1`（2880 bits = 360 bytes）、`bitmap_start = 2`、`dir_start = 3`、`data_start = 19`。
- データ領域は block 19..2879 の2861 blocks（1,464,832 bytes）。ブートセクタからディレクトリまでの19 blocks（9,728 bytes）はビットマップで使用中として保持される。
- `HELLO.TXT`（600 bytes）を新規作成すると、first-fit で block 19..20 を占有する（`start_block = 19`、`size_bytes = 600`）。block 20 の byte 88..511 は0埋めされる。エントリは status `1`、name `"HELLO   TXT"` を持つ。
- 256個すべてのスロットを使うと257個目の create は失敗する。占有領域が重なる2ファイル、小文字を含む name、署名の無い block 0、使用中領域の0 bit、padding bit の1 はいずれも mount 拒否の対象である。
- delete はエントリを全0にしてからビットを0にする。逆順では異常終了時に mount 拒否（§7-9）となるため、逆順で書いてはならない。

## 11. ツール

`tools/dsfs/dsfs.py` が本仕様の host 側リファレンス実装である（Python 標準ライブラリのみ）。

```text
dsfs.py mkfs IMAGE [--sectors N] [--time UNIX]  format（未存在 image は --sectors 必須）
dsfs.py info IMAGE                              スーパーブロックと空き状況
dsfs.py ls IMAGE                                ファイル一覧
dsfs.py put IMAGE FILE [NAME] [--time UNIX]     作成・置き換え（NAME 省略時は FILE 名）
dsfs.py get IMAGE NAME OUTPUT                   展開
dsfs.py cat IMAGE NAME                          stdout へ出力
dsfs.py rm IMAGE NAME                           削除
dsfs.py rename IMAGE OLD NEW                    名前変更
dsfs.py verify IMAGE                            §7 の検証。違反があれば列挙して終了コード1
```

- 書き込み系コマンドは保存前に §7 の mount 検証を通し、操作は §8 の更新順序に従う。違反・形式エラーは `dsfs: ...` を stderr に出して終了コード1。
- `ls` / `info` / `verify` は leak を件数として報告するが、エラーにはしない。
- テストは `python3 -m pytest tools/dsfs/test_dsfs.py`。

## 12. ゲストライブラリ

ゲストは `sol/cpt32/idc.sol` と `sol/cpt32/dsfs.sol` で DSFS を扱う。両ライブラリは名前空間を宣言しているため、関数・定数は `idc.` / `dsfs.` を付けて参照する。

- `sol/cpt32/idc.sol`（名前空間 `idc`）: IDC ドライバ。DMAC channel 0 と IDC `DATA` endpoint による polled 転送で、`idc.idc_enable` / `idc.idc_identify` / `idc.idc_read` / `idc.idc_write` / `idc.idc_error_code` を提供する。転送関数は 0=成功、1=DMAC error、2=IDC error を返す。IRQ は使わない。
- `sol/cpt32/dsfs.sol`（名前空間 `dsfs`）: DSFS 操作。`dsfs.dsfs_mount` が LBA 1 のスーパーブロックを読んで §7 相当の検証と geometry のキャッシュを行い、`dsfs.dsfs_dir_load` / `dsfs.dsfs_find` / `dsfs.dsfs_count` / `dsfs.dsfs_read_file` / `dsfs.dsfs_write_file` / `dsfs.dsfs_delete_file`、エントリ accessor、bitmap 操作を提供する。書き込みは §8 の更新順序に従い、最終 sector の残り byte は in-place で0埋めする。新規/更新エントリの timestamp は `dsfs.dsfs_set_time` の指定値（既定0）である。
- バッファ契約: mount/ディレクトリ buffer は8192 bytes 以上（同一 buffer を `dsfs.dsfs_mount` と `dsfs.dsfs_dir_load` に流用できる）、bitmap buffer は `dsfs.dsfs_bitmap_blocks*512` bytes（16 MiB までの volume なら512 bytes）、file buffer は `sectors*512` bytes。
- エラー値: `dsfs.DSFS_OK` 0、`dsfs.DSFS_ERR_IO` 1、`dsfs.DSFS_ERR_MAGIC` 2、`dsfs.DSFS_ERR_VERSION` 3、`dsfs.DSFS_ERR_LAYOUT` 4、`dsfs.DSFS_ERR_NOT_FOUND` 5、`dsfs.DSFS_ERR_FULL` 6、`dsfs.DSFS_ERR_ARGS` 7。

回帰テストは `tools/dsfs/dsfs_smoke.sol`（ビルド手順はファイル冒頭）と `tests/dsfs_guest.rs`（`cargo test --test dsfs_guest`、`dsfs_guest.bin` のローカルビルドを必要とする）。
