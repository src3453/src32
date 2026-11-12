# SGUB v4 音楽ストリーム仕様

状態: **Draft（実装済み）**。v4 は v3 の operation 形式（32-bit address）と pool を引き継ぎ、次の 3 点を加える。

1. **34SI22 の再生**: target 11 で 34SI22（SGU v3、MMIO `0x80022000`）のレジスタへ書き込む。標本データは従来どおり共有 PCM RAM（target 2）に置く。
2. **フレーム周期**: ヘッダーの `frame_period` が 0 なら従来の VSync（IRQ0、60 Hz）でフレームを進め、0 以外なら PeC タイマの周期 IRQ（IRQ4）で `frame_period` サイクル（48 MHz）ごとにフレームを進める。
3. **複数の拡張カード**: target 3..10（slot 0..7）を全 slot で使えるようにし、各 slot に必要なカードをヘッダー直後の**カード要件表**で宣言する。3HS816 カード（[3HS816 の CPT32 カード](spec_3HS816.md#cpt32-拡張カード)）と 3WS88PWN4 カードを同じ形式で扱う。

v1/v2/v3 の読み取り契約とバイト列は変更しない。reader は version `4` を明示的に扱い、未知の version を拒否する。

## バイト順と上限

複数 byte 整数はすべて big endian。最大ファイルサイズは v1〜v3 と同じ `0x00E00000` byte（14 MiB、`0x00200000` へのロードで 16 MiB RAM に収まる）。ヘッダーと各表の offset はファイル先頭基準。

## ヘッダー（48 byte）

| Offset | Size | 内容 |
|---:|---:|---|
| 0 | 4 | ASCII `SGUB` |
| 4 | 1 | Version: `4` |
| 5 | 1 | Voice count: `16`（SGU 2 基のチャンネル数。v1〜v3 と同じ） |
| 6 | 1 | Flags: bit 0 は全 frame 列を先頭から繰り返す。その他は 0 |
| 7 | 1 | Reserved: 0 |
| 8 | 4 | Frame count。1 以上 |
| 12 | 4 | Pool entry count。0〜65536 |
| 16 | 4 | 初期化操作数 |
| 20 | 4 | Pool directory offset |
| 24 | 4 | 初期化操作列 offset |
| 28 | 4 | Frame 列 offset |
| 32 | 4 | ファイル全体の正確な byte 数 |
| 36 | 2 | Header size: `48` |
| 38 | 1 | カード要件数 `C`（0〜8） |
| 39 | 1 | Reserved: 0 |
| 40 | 4 | `frame_period`: 0 = VSync、それ以外は 1 フレームの長さ（48 MHz サイクル数、`48000`〜`48000000`） |
| 44 | 4 | Reserved: 0 |

`frame_period` の範囲は 1 kHz〜1 Hz。0 は v1〜v3 と同じ VSync 進行。`800000` は 60 Hz だが VSync ではなくタイマで進む（VSync と位相が揃わない）。

## Canonical layout

セクションは次の順で隙間なく置く。

```text
header (48) → カード要件表 (C × 32) → pool directory → pool payloads (ID順) → initial operations → frames
```

Pool directory offset は `48 + C × 32`。pool directory 以降の形式（16 byte entry、ID 順の連続 payload、codec 0 のみ）は v2 と同じ。

## カード要件表

1 entry 32 byte。slot の昇順に並べ、同じ slot を 2 回書かない。

| Entry offset | Size | 内容 |
|---:|---:|---|
| 0 | 1 | Slot: 0〜7 |
| 1 | 1 | ID 長 `L`: 1〜16 |
| 2 | 2 | Reserved: 0 |
| 4 | 4 | ID offset（slot 相対）。`offset + L <= 0x02000000` |
| 8 | 4 | Bulk window 先頭（slot 相対） |
| 12 | 4 | Bulk window 長。0 は WRITE のみ。`start + length <= 0x02000000` |
| 16 | 16 | ID bytes。`L` byte 以降は 0 |

- target 3+n（slot n）への操作は、slot n の entry がある場合だけ有効。entry のない slot への操作は拒否する。entry があって操作のない slot は許す（カード ID の確認だけが行われる）。
- **ID 確認**: MMIO で再生する player は、初期化操作を実行する前に、各 entry について slot base + ID offset から `L` byte を読み、ID bytes と一致することを確認する。不一致ならどのデバイスにも書かずにエラーとする。空 slot の読み出しは 0 なので、未装着も不一致として検出される。出力 callback を使う player は callback 側の責任とする。
- **Bulk window**: FILL と COPY_POOL の destination は、その slot の bulk window の内側に収まらなければならない。WRITE は slot aperture（`0x00000000..0x01FFFFFF`）全体に使える。register/strobe 領域を bulk window に含めてはならない。

既知のカード（exporter が出力する値）:

| カード | ID offset | ID bytes | Bulk window |
|---|---:|---|---|
| 3HS816 | `0x4F8` | `33 48 53 38 31 36 01`（`3HS816` と major version 1） | なし（長さ 0） |
| 3WS88PWN4 | `0x4003F0` | ASCII `3WS88PWN4` | `0x000000`、長さ `0x400000`（PCM RAM） |

reader は ID bytes の意味（カードの種類）を推測しない。表に従って照合と範囲検査だけを行う。

## Target

| Target | Destination | Address range | FILL / COPY_POOL |
|---:|---|---|---|
| 0, 1 | SGU0 / SGU1 | `0x000..0x8FF` | 波形 SRAM `0x000..0x7FF` のみ（v2 と同じ） |
| 2 | 共有 PCM RAM | `0x00000..0xFFFFF` | 全域 |
| 3..10 | 拡張 slot 0..7 | `0x00000000..0x01FFFFFF` | カード要件表の bulk window 内のみ |
| 11 | 34SI22 レジスタ | `0x00..0x3F` | 不可（WRITE のみ） |
| 12..255 | 予約 | — | 拒否 |

slot n の host address は `0xA0000000 + n × 0x02000000 + address`。34SI22 の host address は `0x80022000 + address`。34SI22 のレジスタと副作用は [34SI22 仕様](spec_34SI22.md) に従う。特に PLAY_CTRL（`0x0C`）や STATUS_CTRL（`0x20`）への書き込みは回数と順序に意味があるので、reader と writer は WRITE を統合・省略しない。34SI22 を使う stream は `sgu.version: 3` の環境を要求する。

## 操作形式

v3 と同じ。

| Opcode | 名前 | 形式 | 長さ |
|---:|---|---|---:|
| 0 | WRITE | `opcode:u8, target:u8, address:u32, value:u8` | 7 |
| 1 | FILL | `opcode:u8, target:u8, address:u32, length:u32, value:u8` | 11 |
| 2 | COPY_POOL | `opcode:u8, target:u8, address:u32, pool_id:u16, source_offset:u32, length:u32` | 16 |

FILL と COPY_POOL の length は 1 以上で、destination の範囲（上表）と pool の範囲を超えてはならない。

## Pool type

| Type | 内容 | 長さ | COPY_POOL 先 |
|---:|---|---|---|
| 1 | SGU wave | 256 | target 0/1 の波形 SRAM（`0x100` 境界、256 の倍数） |
| 2 | PCM bytes | 1〜`0x100000` | target 2 |
| 3 | カード RAM image | 1〜`0x02000000` | target 3..10 の bulk window |

34SI22 の標本（PCM8／PCM16／SQV4）は type 2 として target 2 にコピーする。

## 再生

1. 全体を検証する（下記）。
2. MMIO player はカード要件表の ID を確認する。
3. 初期化操作を順に一度実行する。
4. `frame_period == 0` なら以後の VSync ごとに、0 以外なら `frame_period` サイクルごとに、frame を 1 つ実行する。初期化から frame 0 までの間隔は VSync 進行では次の VSync まで、タイマ進行では `frame_period` サイクル。
5. 末尾 frame の次の進行時、repeat flag があれば frame 0 に戻る。なければ停止処理をして終了する。

停止処理は v1〜v3 と同じく SGU 16 チャンネルの音量と PCM 制御を 0 にし、さらに stream が target 11 を含む場合は 34SI22 の PLAY_CTRL（`0x0C`）に 0 を書いて両チャンネルを停止する。拡張カードは種類が分からないので触らない。カードの消音が必要な場合、writer は最後の frame に消音操作を入れる（Furnace exporter はそうする）。repeat 時に初期化列は再実行しない。

### CPT32 guest driver のタイマ設定

`sol/cpt32/sgu_music_driver.c` は `frame_period != 0` のとき PeC タイマ 0 を次のように設定し、IRQ0 は使わない。

| 手順 | 書き込み |
|---|---|
| 1 | `CTRL`（`0x80040400`）← `0x10`（RESET：カウンタ・分周・pending をクリア） |
| 2 | `PRESCALER`（`0x80040420`）← 0 |
| 3 | `COMPARE_HI/LO`（`0x80040414` / `0x80040410`）← 0 / `frame_period` |
| 4 | `RELOAD_HI/LO`（`0x8004041C` / `0x80040418`）← 0 / `frame_period` |
| 5 | `TIMER_ENABLE`（`0x80040184`）の bit 0 を立てる |
| 6 | `CTRL` ← `0x0D`（periodic、ENABLE、IRQ_ENABLE） |

IRQ4 の handler は `TIMER_ACK`（`0x8004018C`）に 1 を書いてから 1 frame を実行する（IRQC の pending は dispatcher が ACK する）。periodic モードは期限ごとに RELOAD を COMPARE へ加算するので、handler の遅れで周期の位相はずれない。handler が周期内に終わらない場合は次の IRQ がその分遅れ、frame は省略されずに順に実行される。再生が終わったら timer 0 の CTRL を 0 にして IRQ4 を無効にする。

## 検証要件

reader はデバイスに書く前に次をすべて確認する。

- ヘッダー: magic、version 4、voice count 16、予約 bit/byte が 0、header size 48、`C <= 8`、`frame_period` が 0 または `48000..=48000000`、frame count が 1 以上、total size がファイル長と一致し 14 MiB 以下、pool directory offset が `48 + 32C`。
- カード要件表: slot が 0〜7 で厳密に昇順、ID 長 1〜16、予約 0、ID の未使用 byte が 0、ID 範囲と bulk window が aperture 内（overflow を含めて検査）。
- pool directory / payload: v2 と同じ canonical 規則。type 1/2/3 の長さ制限。
- 操作: 未知 opcode、予約 target、範囲外 address、entry のない slot への操作、target 11 への FILL/COPY_POOL、bulk window 外の FILL/COPY_POOL、pool type と target の不一致、0 長、overflow をすべて拒否する。
- frame 列: 切り詰めと末尾の余剰 byte を拒否する。

v1/v2/v3 の reader 制限と実行経路は変更しない。

## Furnace exporter

Furnace fork（`../furnace`）の **File > Export > ROM > CPT32 SGUB** と CLI `furnace -romout song.sgub song.fur` は、曲の chip 構成で version を選ぶ。

| 構成 | 出力 |
|---|---|
| S3W2 1〜2 基だけ、VSync 進行 | v2（従来どおり） |
| S3HS 1 基だけ、VSync 進行 | v3（従来どおり。slot 0） |
| それ以外（34SI22、3HS816、複数カード、組み合わせ、タイマ進行） | v4 |

v4 で受け付ける chip は S3W2 0〜2 基、34SI22 0〜1 基（34SI22A も可。ただし FIFO DAC を使う書き込み、つまり PLAY_FORMAT 5 と `0x25..0x2C` が現れたら export を失敗させる。CPT32 の SGU v3 には FIFO がない）、拡張カード chip（S3HS = 3WS88PWN4 カード、3HS816 = 3HS816 カード）合わせて 0〜8 基。カード chip は曲の system 順に slot 0 から割り当てる。

- **進行**: `-romconf clock=auto|vsync|timer`（既定 `auto`）、`rate=<Hz>`（既定は曲の tick rate）。`auto` は rate が 60 Hz なら VSync、それ以外ならタイマ。`frame_period = round(48000000 / rate)`。書き込みは 48 kHz の累積時間（1 sample = 1000 サイクル）で、そのとき属する frame（`floor(t / frame_period)`）へ量子化する。途中で tick rate が変わる曲も、時刻は保たれ、frame 内の間隔だけが失われる。
- **34SI22**: Furnace の sample memory（PCM8 は符号なし、PCM16 は big endian、SQV4 はヘッダ付き）をそのまま type 2 pool にし、S3W2 の PCM の後ろ（16 byte 境界、1 byte 以上の無音ガードを空ける）にコピーする。START/STOP（`0x00..0x0B`）の書き込みは配置先へ移し、3 byte をまとめて書き直す（桁上がりを伝えるため）。共有 PCM RAM 1 MiB を超える場合は拒否する。frame 0 で PLAY_CTRL を 0、STATUS_CTRL に `0xF4`（sticky bit のクリア）を書き、Furnace の reset 直後の音量・分周・形式を書き直す。
- **3HS816**: frame 0 で Furnace の reset 直後のレジスタ状態（`0x000..0x4F7`、Gate を除く。ミュート `0x480..0x48F` は GUI のミュートに関係なく 0）を書き、全 Gate に 0 を書く。repeat でも同じ状態から始まる。非 repeat の stream は最後に全チャンネルの Gate を 0 にする frame を加える。
- **S3HS（v4）**: v3 と同じく PCM RAM image を type 3 pool として bulk window へコピーし、レジスタ書き込みを slot 相対 `0x400000 + addr` へ出す。非 repeat の stream は最後に HS 8 チャンネルの Gate を 0 にする。

## ImHex パターン

v4 用の pattern はまだない。`tools/imhex/sgub_v2.hexpat` は v4 のヘッダー拡張とカード要件表を解釈しない。
