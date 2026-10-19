# SGUB v2 音楽ストリーム仕様案

状態: **Draft（実装済み）**。Rust の reader/compiler、Furnace S3W2 ROM exporter、CPT32 guest C driver が v2 の WRITE/FILL/COPY_POOL に対応する。v3はoperation addressを32-bitへ拡張し、拡張カードslot 0への出力を追加する。Music Editorからのv2編集・exportは対象外。v1との互換契約は維持する。

## 目的

v1 は PCMRAM や波形 SRAM への1 byte 書き込みも5 byteレコードにするため、大きなデータを初期化するとファイルが大きくなる。v2 はデータを pool に一度だけ格納し、初期化またはフレームの操作から pool ID で参照する。さらに連続領域の fill 操作を設け、PCM の無音領域を繰り返し列挙せず初期化できるようにする。

v2 reader は version byte `2` を要求する。v1 の version `1`、ヘッダー、レコード解釈は変更しない。reader は両方を明示的にサポートし、未知の version を拒否する。

## バイト順と上限

すべての複数 byte 整数は big endian。最大ファイルサイズは v1 と同じ `0x00E00000` byte (14 MiB)。この上限は stream を主 RAM の `0x00200000` からロードし、16 MiB RAM に収める既存 driver 契約を保つ。

ヘッダーとdirectoryのoffsetはすべて**ファイル先頭基準**。guest driverが読み出すときは `STREAM_BASE + offset` に変換する。directory参照、初期化列、frame列、pool payloadすべてに適用し、offsetをそのままCPUアドレスとして使わない。

## ヘッダー

ヘッダーは40 byte。予約 field はすべて0でなければならない。

| Offset | Size | 内容 |
|---:|---:|---|
| 0 | 4 | ASCII `SGUB` |
| 4 | 1 | Version: `2` |
| 5 | 1 | Voice count: `16` |
| 6 | 1 | Flags: bit 0 は全 frame sequence を先頭から繰り返す。その他は0 |
| 7 | 1 | Reserved: 0 |
| 8 | 4 | Frame count。1以上 |
| 12 | 4 | Pool entry count。0〜65536 |
| 16 | 4 | 初期化操作数 |
| 20 | 4 | Pool directory offset。canonical layout では40 |
| 24 | 4 | 初期化操作列 offset |
| 28 | 4 | Frame 列 offset |
| 32 | 4 | ファイル全体の正確な byte 数 |
| 36 | 2 | Header size: `40` |
| 38 | 2 | Reserved: 0 |

## Canonical layout と pool directory

セクションは次の順で隙間なく置く。

```text
header → pool directory → pool payloads (ID順) → initial operations → frames
```

Pool directory entry は16 byte。

| Entry offset | Size | 内容 |
|---:|---:|---|
| 0 | 2 | Pool ID。0から連続する番号 |
| 2 | 1 | Type: 1 = SGU wave (256 byte)、2 = PCM bytes |
| 3 | 1 | Codec: 0 = raw。v2 初版では他の値を拒否 |
| 4 | 4 | 展開後 byte 数 |
| 8 | 4 | 格納 byte 数 |
| 12 | 4 | ファイル先頭からの payload offset |

Payload は directory の直後から ID 順に連続配置し、codec 0 では格納 byte 数と展開後 byte 数が等しい。Wave entry は正確に256 byte。PCM entry は1〜`0x100000` byte。Pool ID は entry の位置と一致し、同じデータを複数箇所で使う場合は同じ ID を参照する。異なる ID の payload 間の重複格納や暗黙の dedup はない。

`initial operations offset` は最後の payload の直後。初期化操作数を decode した終端は `frames offset` と一致する。frame 列は最終 frame の終端がファイル終端と一致し、追加 byte を許さない。すべての offset、加算、乗算は overflow 検査を行う。

## 操作形式

初期化列はヘッダーの初期化操作数だけ decode する。各 frame は big-endian `u16 operation_count` と、その数の操作を持つ。操作は opcode を含む固定長形式。

### WRITE — opcode 0、合計6 byte

```text
opcode:u8, target:u8, address:u24, value:u8
```

Target 0/1 は SGU0/SGU1 offset `0x000..0x8FF`、target 2 は共有 PCMRAM offset `0x00000..0xFFFFF`。書き込み順と重複書き込みの副作用を保つ。

### FILL — opcode 1、合計10 byte

```text
opcode:u8, target:u8, address:u24, length:u32, value:u8
```

連続範囲を指定値で埋める。Length は1以上で、target の範囲を超えてはならない。Target 0/1 は波形 SRAM `0x000..0x7FF` のみ許し、制御レジスタ `0x800..0x8FF` への一括書き込みを禁止する。Target 2 は PCMRAM 全域を許す。これは初期化時の PCMRAM silence fill (`0x80`) などに使う。

### COPY_POOL — opcode 2、合計15 byte

```text
opcode:u8, target:u8, address:u24, pool_id:u16,
source_offset:u32, length:u32
```

指定 pool entry の `[source_offset, source_offset + length)` を target の address からコピーする。Length は1以上。Pool の source 範囲と target の destination 範囲をどちらも超えてはならない。Type 1 wave は target 0/1 の波形 SRAM にのみコピーでき、address は `0x100` 境界、length は256 byteの整数倍とする。Type 2 PCM は target 2 にのみコピーできる。将来 type/codec を追加する場合は version 2 の互換規則を更新し、既知値として安全に解釈できない reader は拒否する。

## 再生

再生開始時に初期化操作を順番に一度実行し、その次の60 Hz VSyncで frame 0 を実行する。その後は VSync ごとに frame を1つ実行する。Frame 内操作もファイル順に実行する。Pool payload 自体はメモリへ暗黙展開されず、WRITE/FILL/COPY_POOL が指定した時点で destination に反映される。

末尾 frame の次の VSync では、repeat flag があれば frame 0 に戻り、なければ channel volume と PCM control を停止状態にする。repeat 時に初期化列を再実行しない。したがってループに必要な SGU state の reset は frame 0 の WRITE/FILL/COPY_POOL に含める。

## 検証要件

Reader はデータを変更する前に、ヘッダー、全 directory entry、全 payload 範囲、初期化操作、全 frame と全操作を走査して検証する。未知 opcode/type/codec、重複または欠落 ID、範囲外 copy/fill、切り詰め、overlap、予約 bit の非0、frame 後の余剰 byte は拒否する。実行時の2回目の decode は検証済み範囲内で行う。

Writer と reader の相互運用テストでは、最低限、v1 との version 分離、重複 pool 参照、同じ wave の複数 destination、PCM pool copy、PCMRAM fill、frame 内の順序、副作用を持つ WRITE の重複保持、全 repeat と非repeat、各種 offset/length overflow、14 MiB 上限を確認する。

## Furnace exporter への適用

Furnace の S3W2 ROM exporter は v2 を出力し、次のpool化を行う。過去のv1ファイルもreaderとguest driverで再生できる。S3HS outputは [SGUB v3](spec_SGUB_v3.md) を使う。

- 同一の256 byte波形を1つの wave pool entry にまとめ、初期化または必要な frame で SGU 波形 SRAM に COPY_POOL する。
- PCM bank を PCM pool entry とし、必要な destination 範囲だけ COPY_POOL する。未使用領域は FILL で `0x80` に初期化して決定性を保つ。
- チップごとに異なる register state、wave RAM、共有 PCM RAM の配置・競合を保持する。Pool ID はデータ内容の共有を表し、PCM destination address の同一性を意味しない。
- v1 と比較する実測ファイルサイズ、波形内容、PCM の signed→unsigned 変換、phase-reset 等の WRITE 順序を regression test に含める。

この案では codec 0 raw のみを必須にする。LZ 系圧縮、delta encoding、pool entry の alignment、内部 loop point、sub-frame timing は v2 初版の範囲外とし、圧縮を入れる場合は実測で raw pool の改善幅を確認してから別途設計する。

## Guest driverのビルドと更新

Guest playback uses `sol/cpt32/sgu_music_driver.c` and the SDK implementation
`sdk/cpt32/src/sgub.c`. From the repository root, build it with:

```sh
python tools/llvm-src32/driver.py sol/cpt32/sgu_music_driver.c \
  sdk/cpt32/src/irqc.c sdk/cpt32/src/sgub.c \
  sdk/cpt32/src/pcg.c -I sdk/cpt32/include -O0 -o sgu_music_driver.bin
```

The music stream stays at `0x00200000`. Music Editor exports this C source next
to the stream; compile it with the SDK sources and include directory above.
Old Sol driver copies must be replaced with the current C driver. SGUB files do
not need to be re-exported.

During playback the driver uses the PCG text screen to show all 16 channels'
volume and frequency registers in hexadecimal. It refreshes the display once
per VSync after applying that frame's stream operations.

初期化は全検証が通ってから実行し、完了後にIRQ0を有効にする。FILL/COPY_POOLは制御レジスタに触れない契約なので32-bitのまとめ読み書きで処理し、端数だけbyte単位で処理する。通常のWRITEは順序と副作用を維持する。現在のSRC32 C backendは最適化ビルドで一部内部呼び出し規約を扱えないため、上記の `-O0` を使う。

## ImHex パターン

`tools/imhex/sgub_v2.hexpat` はヘッダー、pool directory と payload、可変長の初期化／frame 操作、各 frame を可視化する。`StreamPlayer` による検証の代替ではない。
