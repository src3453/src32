---
lang: ja
translation_of: ../spec_WAX.md
---
# WAX（Where's All the eXecutables?）コンテナ仕様 v1

WAX は SRC32 用の小型 ELF 風コンテナです。拡張子:
`.wax` = 実行ファイル、`.wal` = ライブラリ（同じ形式で、型が異なります）。
WAX は "Where's All the eXecutables?" の略です。
WAL は "Where's All the Libraries?" の略で、v1 では再配置を含まない PIC lump です。
この仕様が対象とするのは v1 コンテナのみです。リンク／再配置はまだありません。

## レイアウト

64 バイトのヘッダーに続いて、text バイト、rodata バイト、debug バイトが並びます。
フィールドの順序とオフセットは固定です。ヘッダー内の複数バイトの
フィールドはすべてリトルエンディアンです。ペイロードのバイト列は不透明です。

```
オフセット  サイズ  名称
0       4     magic = 57 41 58 00 ("WAX\0")
4       2     version = 1
6       2     type: 1 = .wax 実行ファイル、2 = .wal ライブラリ
8       2     arch = 1 (SRC32)
10      2     flags: bit0 = DEBUG_PRESENT、その他は 0
12      4     header_size = 64
16      4     base_addr（実行ファイルのロード先。 .wal では無視）
20      4     entry_offset（entry = base + entry_offset。 .wal では無視）
24      4     text_offset
28      4     text_size（0 より大きいこと）
32      4     rodata_offset
36      4     rodata_size（0 は存在しないことを示す）
40      4     debug_offset
44      4     debug_size（0 は存在しないことを示す）
48      16    reserved（0）
```

## 規則

- オフセットは 4 バイト境界に揃えます。データブロックはファイル内に収まり、
  重複せず、EOF を越えてラップしません。
- text_size > 0。rodata/debug は空でもかまいません（size 0 の場合、
  offset は無視されます）。
- debug_size > 0 の場合に限り DEBUG_PRESENT を設定します（かつ、その場合は必ず設定します）。
- base_addr + text/rodata の範囲が u32 の範囲を超えてはなりません。
  Main RAM は 0x00000000-0x00FFFFFF（16 MB）です。
- .wal: base_addr と entry_offset は 0 でなければなりません（この形式では無視されます）。
  text/rodata は PIC データ塊であり、v1 では再配置を行いません。
- .wax の entry = base_addr + entry_offset は、ロードされた text 範囲内に
  位置しなければなりません。
- 未知の version/arch/type/flag bits は拒否します。
- v1 にはチェックサムがありません。破損チェックには magic +
  version + 範囲検証を使います。
- v1 では Debug block は NOLOAD かつ不透明です。ローダーはこれをスキップします。

## ロード

1. 先頭 4 バイトが magic と異なる場合、入力を legacy raw
   .bin として扱います。address 0 にロードし、PC = 0 とします。
2. それ以外はヘッダーを解析して検証し、text バイトを
   base_addr+i に、rodata バイトを base_addr+text_size+i に書き込みます
   （rodata はメモリ上で text の後に続きます）。
3. PC = base_addr + entry_offset に設定します（.wax のみ）。
4. CPT32PKG entrypoint path は .wax または .bin を指定できます。
   ローダーは magic を判別します。Manifest schema は変更されません。

## テストベクトル

- 最小 .wax: text = 4 バイトの NOP (0x00000000)、
  base 0、entry_offset 0、rodata/debug なし。0 にロードされ、PC=0。
- オフセット付き .wax: base 0x100 の text、entry_offset 0。
  ローダーは 0x100 に書き込み、PC=0x100。
- .wal: type=2、base=entry=0、text あり。解析 OK、
  実行ファイルとしてのロードは拒否されます（ライブラリであり、実行不可）。
- 拒否: 不正な magic version、重複、アラインメントされていない offset、
  空の text、text 外の entry、切り詰められたデータブロック。

## ツール（v1）

- tools/wax/wax.py: .wax/.wal の pack/unpack/info。
- src/wax.rs: main/testbench/monitor が使用する parse + load。
- asm.py は変更なし（引き続き raw .bin を出力し、wax.py が合成します）。
