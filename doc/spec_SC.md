# SC (Sprite Controller) 仕様

## 1. 位置付け

SCはスプライト属性を読み、VRAM上の画像を2Dラスタライズする描画供給元である。出力先はVDPの `GP0`～`GP7` のうち1つを選ぶ。SCだから常にPCGやビットマップより前面になるわけではなく、相対的な前後は出力GP番号で決まる。GP番号が同じ場合はSCがそのGPの供給元として合成される順で決まる（初版の供給元登録順ではSCを最後に合成する）。

SC内部のsprite priority/indexは同一SC出力内の順序のみを決め、GP順は変更しない。これにより、PCG→SC→前景PCGのような構成は、PCGを異なるGPへ出して実現する。

## 2. 初版機能

- VRAM内Sprite Attribute Table (SAT)からスプライト属性を取得する。
- VRAM上の4bpp indexed patternを読み、64色CLUTでRGBへ変換する。
- X/Y位置、幅/高さ、H/V flip、整数拡大率、透明インデックス、enableを扱う。
- SC内priorityで重なりを決定し、指定GPへ透明度付き画素を出力する。
- 画面外部分はクリップする。画面外座標や無効なpattern範囲を理由にメモリアクセスを画面外へ行わない。

初版では回転、半透明個別指定、衝突判定、スキャンラインsprite上限、BitBltを実装しない。BitBltはSCの画素合成とは別の転送エンジン仕様として扱う。回転は将来拡張であり、初版予約フィールドを0にする。

## 3. 制約と画素形式

| 項目 | 初版仕様 |
|---|---|
| 表示領域 | 320×240 |
| 最大登録数 | 256 sprites |
| SAT entry | 16 bytes、big-endian |
| 位置 | signed 16-bit X/Y、画面左上原点、右/下が正 |
| サイズ | 8-bit幅/高さ、1～64 texels |
| Pattern | 4bpp packed indexed pixels、左側pixelを上位nibble |
| Palette | 6-bit CLUT index。palette base + texel indexをmodulo 64で参照 |
| 透明 | texel index 0を透明。透明画素は下位spriteを覆わない |
| 拡大 | X/Y各1～4倍のnearest-neighbor整数拡大 |
| 合成 | SC内の低priorityから高priorityへ。priority同値なら小さいsprite indexから大きいindexへ描き、大きいindexが前面 |

4bppのpattern byte数は `ceil(width * height / 2)`。行ごとにbyte境界へ丸めず、全pixelを連続してpackする。パターン開始アドレスはVRAM相対byte offsetであり、範囲がVRAM外へ出るspriteはそのsprite全体を無効にしてSCエラーを記録する。

## 4. Sprite Attribute Table

各entryは16 bytes。SAT baseは16-byte alignedとする。

| Offset | Size | 内容 |
|---:|---:|---|
| `0x00` | 2 | X: signed 16-bit |
| `0x02` | 2 | Y: signed 16-bit |
| `0x04` | 1 | width texels (1～64) |
| `0x05` | 1 | height texels (1～64) |
| `0x06` | 4 | pattern VRAM byte offset |
| `0x0A` | 1 | palette base (下位6bit) |
| `0x0B` | 1 | priority (0～255) |
| `0x0C` | 1 | scale X (1～4、0は無効entry) |
| `0x0D` | 1 | scale Y (1～4、0は無効entry) |
| `0x0E` | 1 | flags: bit0 enable, bit1 H flip, bit2 V flip |
| `0x0F` | 1 | reserved、書込み0、読出し0 |

不正なサイズ、scale、未定義flag bitを持つentryはそのspriteだけを描画せず、INVALID_SPRITEをstickyに立てる。disabled entryは検査対象外。SAT base+count×16がVRAM範囲外ならSC設定エラーとしてSC全体を描画しない。

## 5. 表示順

同じ画素を覆うスプライトは、priority昇順、次にsprite index昇順で処理する。後に処理した不透明pixelが手前となる。したがって高priorityが手前、priority同値ではindexの大きいentryが手前となる。透明index 0は書き込みを行わず、背後のSC spriteまたは同一GPの下位供給元を保持する。

## 6. MMIO

SC MMIO baseは `0x80010000`、sizeは64 KiB。レジスタは32-bit big-endian、4-byte aligned。byte accessはレジスタ内の対応byte laneに反映する。未定義領域は0を読み、書込みを無視する。

| Offset | Register | Access | 定義 |
|---:|---|:---:|---|
| `0x0000` | ID | R | `0x53433031` ("SC01") |
| `0x0004` | CONTROL | RW | bit0 enable、bit1 soft reset pulse |
| `0x0008` | STATUS | R/W1C | bit0 BUSY、bit1 INVALID_CONFIG、bit2 INVALID_SPRITE |
| `0x0010` | SAT_BASE | RW | VRAM相対byte offset、16-byte aligned |
| `0x0014` | SPRITE_COUNT | RW | 0～256 |
| `0x0018` | OUTPUT_GP | RW | 0～7 |

RESET後はSC disabled、sprite count=0、SAT base=0、output GP=0、status clear。設定値は即時反映し、frame latch/double bufferingは初版に含めない。reserved control bitは0を書き、1を書いた場合はINVALID_CONFIGを記録する。

## 7. VRAM共有と同期

SCはVDPと同一4 MiB VRAMを共有する。SAT/pattern readはVDP VRAMの相対offsetを用いる。CPUが描画中に属性やpatternを書き換えた場合、次に評価する画素から新しい値を参照する。エミュレータはフレーム取得時にスプライト一覧を読む実装でもよいが、その場合は変更が次フレームから反映される差を実装上の制約として明記する。

## 8. 初版に含めない機能

任意角度回転、fractional scale、sprite collision、line limit/flicker、per-sprite alpha/blending mode、複数GPへの同時出力、BitBltは予約または別仕様とする。将来拡張でも既存entryの予約byte/bitを再利用する場合は、feature/version識別を追加してから有効化する。
