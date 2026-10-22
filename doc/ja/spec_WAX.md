---
lang: ja
translation_of: ../spec_WAX.md
---
# WAX（Where's All the eXecutables?）コンテナー仕様 v2

このページは実装済みの v2 形式です。別仕様の [WAX/WAL v3](spec_WAX_v3.md) は
CPU 保護の必須化、依存関係、ライブラリシンボルを定義します。v3 の生成・ロードは未実装です。

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
10            2     flags: bit0 = DEBUG_PRESENT、bit1 = PROTECTED、その他は 0
12            4     header_size = 64
16            4     base_addr: .wax の最小 block address、.wal は 0
20            4     entry_offset: entry = base_addr + entry_offset、.wal は 0
24            4     block_table_offset
28            4     block_count
32            4     block_entry_size = 12（PROTECTED 時は 16）
36            4     debug_offset（存在しない場合は 0）
40            4     debug_size（存在しない場合は 0）
44            4     file_size（実際の byte 数と一致）
48            4     user_stack_top（PROTECTED 時のみ。それ以外は 0）
52            4     user_stack_size（PROTECTED 時のみ。それ以外は 0）
56            8     reserved（0）

Block table entry（各 12 byte。PROTECTED 時は 16 byte）:
0             4     load_address: .wax の u32 address、.wal の PIC offset
4             4     file_offset: コンテナー内の payload 位置
8             4     size: payload byte 数
12            4     perm（PROTECTED 時のみ）: bit0 R、bit1 W、bit2 X
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
- `PROTECTED` の規則は下の「保護付き実行ファイル」を参照してください。

## 保護付き実行ファイル

`PROTECTED`（flags bit1）は、拡張 P の **ユーザーモード**
（[[doc/ja/spec_CPU_priv|拡張P仕様]]）で動く `.wax` を示します。フラグのない image は
変わらず、従来どおりスーパーバイザーモードで動きます。version は上げません。
このフラグを知らない v2 loader はファイルを拒否するので、保護付きプログラムが
誤って保護なしで動くことはありません。

`PROTECTED` を設定したときの規則:

- type は `.wax` に限ります。`PROTECTED` 付きの `.wal` は拒否します。
- `block_entry_size` は 16 です。各 entry の `perm` は R（1）、W（2）、X（4）の
  0 以外の組み合わせで、それ以外の bit は拒否します。`perm` は拡張 P の
  領域 `FLAGS` と同じ bit です。
- `block_count` は最大 7 です（領域 0 がスタック、領域 1..7 が block）。
- `user_stack_top` と `user_stack_size` は 0 以外の 4 の倍数です。スタックは
  `user_stack_top - user_stack_size .. user_stack_top` です。address 0 を下回らず、
  block と重ならず、終端は `0xFFFF0000` 以下でなければなりません。
- どの block も CPU 予約領域 `0xFFFF0000..` にかかってはいけません。
- entry は X を持つ block の中に置きます。
- header の 56..63 byte は 0 のままです。`PROTECTED` がなければ 48..63 byte は 0、
  `block_entry_size` は 12 のままです。

### ユーザーモードへの受け渡し

マシンは常にスーパーバイザーモードでリセットします。ユーザーモードに入るのは
**BIOS の WAX loader** です。保護付き image では次を行います。

1. ファイルを検証し、各 block を `load_address` に書き込みます。
2. 保護領域（`RGNn_START/END/FLAGS`）を設定します。
   - 領域 0: ユーザースタック、`R|W`
   - 領域 `1 + i`: テーブル順で i 番目の block、その block の `perm`
   - それ以外の領域: すべて 0（無効）
3. `USP = user_stack_top`、`EPC = entry`、`STATUS.PU = 1` にします。`PIE` で
   ユーザーモード中の割り込み許可を選びます。
4. `IRET` を実行します。

payload block はファイルからコピーするだけで、スタックや gap は消去しません。
デバイスは開放しません。保護付きプログラムは `SYSCALL` でデバイスを使い、
そのサービスは SYSCALL ベクターを設定した BIOS／OS が定義します。

現状:

- `src/wax.rs` と `tools/wax/wax.py` は保護付き image を解析・検証し、領域一覧
  （`WaxImage::user_regions`、`user_regions(image)`）を提供します。
- BIOS の WAX loader はまだ実装されていません。ホストの直接ロード（main、
  testbench、monitor）はスーパーバイザーモードでしか起動しないため、保護付き image は
  保護なしで動かさずエラーで拒否します。BIOS ROM image 自体は `PROTECTED` にできません。
- `sol/cpt32/wax.sol`（DOS の `RUN`）は未知のフラグとして拒否します。

## ロード

1. WAX magic がない入力は従来どおり raw `.bin` とし、address 0 にロードして PC=0 にします。
2. WAX は各 payload をテーブルの `load_address` に書き込みます。アドレス間の hole は
   ファイルに含まれず、loader も書き込みません。
3. `.wax` は `base_addr + entry_offset` から実行し、debug はスキップします。
4. CPT32PKG entrypoint は `.wax` または raw `.bin` を指定できます。Manifest は変更しません。

## テストベクトル

- 保護付き: `0x100000` に `R|X` の code、`0x110000` に `R|W` の data、スタック
  `0x1FF000..0x200000`。領域はスタック、code、data の順です。entry block に X がない、
  permission が 0 または未知の bit、スタックが block や CPU 予約領域と重なる、
  スタックの非アライン、8 block 以上、`.wal` を拒否します。

- 1 block: address 0 に 4 byte NOP、`base_addr=0`、`entry_offset=0`。
- sparse image: `0x100` に NOP、`0x10000` に HALT。各 4 byte の 2 block は合計 96 byte
  （64 byte header + 24 byte table + 8 byte payload）で、間のゼロ領域を保存しません。
- 拒否: 空テーブル、順不同／重複するロード範囲、hole 内 entry、テーブル／payload 重複、
  ファイル範囲外 payload、address wrap、不正 version。
- `.wal`: type=2、base=entry=0、1個以上の PIC block。実行は拒否します。

## ツール

- `tools/wax/wax.py pack --base 0x100 --block 0x100=code.bin
  --block 0x10000=stub.bin -o app.wax` で複数 block を pack できます。
- `tools/wax/wax.py pack --protected --base 0x100000
  --block 0x100000=code.bin:rx --block 0x110000=data.bin:rw
  --stack-top 0x200000 --stack-size 0x1000 -o app.wax` で保護付き image を作れます。
  `info` はスタックと各 block の permission を表示します。
- `tools/asm/asm.py input.asm -o output.wax [--entry LABEL]` は `.ORG` の address region ごとに
  block を出力します。現在の PC と同じ address への `.ORG` は、出力済み byte の開始 address を
  変更しません。gap は省略されますが、block を load address に配置してゼロ埋め展開すると
  raw binary の出力と一致します。`.wax` 拡張子で WAX を選択し、その他は raw binary のままです。
  `.PROTECTED stack_top, stack_size` で `.wax` を PROTECTED にし、`.PERM rwx` は隣接する address でも
  その権限の新しい block を開始します（[CPU §7.1](spec_CPU.md)）。`solc` は
  `!abi_compat "Foundation"` でこれらを出力します。
- `src/wax.rs` は block を解析・ロードします。main、testbench、monitor、BIOS は各 block の
  load address を使います。
- `sol/cpt32/wax.sol` は guest RAM 上の WAX v2 実行ファイルを検証し、各 sparse
  block をロードして宣言された entry へ制御を移します。DOS では `RUN NAME`
  で起動します。DOS のファイル上限は 64 KiB です。
  `0x00FE0000..0x01000000` と重なる block は拒否します。この RAM は image の staging、
  8 KiB の file-span bitmap、および転送 trampoline に予約されています。
- `tools/imhex/wax.hexpat` は v2 header、block table、file offset 上の payload、
  任意の debug block を表示します。
