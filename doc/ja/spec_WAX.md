---
lang: ja
translation_of: ../spec_WAX.md
---
# WAX（Where's All the eXecutables?）コンテナー仕様 v2

WAX は SRC32 のスパース実行形式です。`.wax` は実行ファイル、`.wal` は再配置を
含まない PIC ライブラリです。ブロックテーブルを使い、未マップのアドレス間隔を
ファイル内に格納しません。v2 は v1 と互換性がなく、v1 ファイルは再 pack が必要です。
v1 互換 parser は提供しません。

## レイアウト

固定 64 バイトのヘッダー、ブロックテーブル、アラインメント済みの payload block、
任意の debug block の順に配置します。複数バイト整数はすべてビッグエンディアンです。
payload の byte 列は不透明です。

```
Header offset  サイズ  名称
0             4     magic = 57 41 58 00 ("WAX\0")
4             2     version = 2
6             2     type: 1 = .wax 実行ファイル、2 = .wal ライブラリ
8             2     arch = 1 (SRC32)
10            2     flags: bit0 = DEBUG_PRESENT、その他は 0
12            4     header_size = 64
16            4     base_addr: .wax の最小 block address、.wal は 0
20            4     entry_offset: entry = base_addr + entry_offset、.wal は 0
24            4     block_table_offset
28            4     block_count
32            4     block_entry_size = 12
36            4     debug_offset（存在しない場合は 0）
40            4     debug_size（存在しない場合は 0）
44            4     file_size（実際の byte 数と一致）
48            16    reserved（0）

Block table entry（各 12 byte）:
0             4     load_address: .wax の u32 address、.wal の PIC offset
4             4     file_offset: コンテナー内の payload 位置
8             4     size: payload byte 数
```

## 規則

- `block_table_offset` は 4 byte 境界かつ 64 以上です。`block_count` は 1 以上、
  `block_entry_size` は 12 固定です。テーブル全体がファイル内に収まる必要があります。
- block size は 0 にできません。テーブルは `load_address` 昇順で、ロード先の範囲が
  重複したり `0x1_0000_0000` を越えて wrap したりしてはいけません。
- `file_offset` は 4 byte 境界で、テーブルの後ろを指します。payload は `file_size` 内に
  収まり、相互および debug block と重複しません。`file_size` は実ファイル長と一致します。
- `.wax` の `base_addr` は先頭 block の `load_address` です。entry
  (`base_addr + entry_offset`) は u32 wrap せず、いずれか 1 block の内部に置きます。
  block 間の hole は entry にできません。
- `.wal` の `base_addr` と `entry_offset` は 0 でなければなりません。block address は
  PIC offset です。ライブラリは解析できますが実行できません。
- `debug_size > 0` の場合に限り `DEBUG_PRESENT` を設定します。debug は NOLOAD で不透明です。
- 未知の version/arch/type/flag bits は拒否します。v2 に checksum はありません。

## ロード

1. WAX magic がない入力は従来どおり raw `.bin` とし、address 0 にロードして PC=0 にします。
2. WAX は各 payload をテーブルの `load_address` に書き込みます。アドレス間の hole は
   ファイルに含まれず、loader も書き込みません。
3. `.wax` は `base_addr + entry_offset` から実行し、debug はスキップします。
4. CPT32PKG entrypoint は `.wax` または raw `.bin` を指定できます。Manifest は変更しません。

## テストベクトル

- 1 block: address 0 に 4 byte NOP、`base_addr=0`、`entry_offset=0`。
- sparse image: `0x100` に NOP、`0x10000` に HALT。各 4 byte の 2 block は合計 96 byte
  （64 byte header + 24 byte table + 8 byte payload）で、間のゼロ領域を保存しません。
- 拒否: 空テーブル、順不同／重複するロード範囲、hole 内 entry、テーブル／payload 重複、
  ファイル範囲外 payload、address wrap、不正 version。
- `.wal`: type=2、base=entry=0、1個以上の PIC block。実行は拒否します。

## ツール

- `tools/wax/wax.py pack --base 0x100 --block 0x100=code.bin
  --block 0x10000=stub.bin -o app.wax` で複数 block を pack できます。
- `tools/asm/asm.py input.asm -o output.wax [--entry LABEL]` は `.ORG` の address region ごとに
  block を出力します。現在の PC と同じ address への `.ORG` は、出力済み byte の開始 address を
  変更しません。gap は省略されますが、block を load address に配置してゼロ埋め展開すると
  raw binary の出力と一致します。`.wax` 拡張子で WAX を選択し、その他は raw binary のままです。
- `src/wax.rs` は block を解析・ロードします。main、testbench、monitor、BIOS は各 block の
  load address を使います。
- `sol/cpt32/wax.sol` は guest RAM 上の WAX v2 実行ファイルを検証し、各 sparse
  block をロードして宣言された entry へ制御を移します。DOS では `RUN NAME`
  で起動します。DOS のファイル上限は 64 KiB です。
  `0x00FE0000..0x01000000` と重なる block は拒否します。この RAM は image の staging、
  8 KiB の file-span bitmap、および転送 trampoline に予約されています。
- `tools/imhex/wax.hexpat` は v2 header、block table、file offset 上の payload、
  任意の debug block を表示します。
