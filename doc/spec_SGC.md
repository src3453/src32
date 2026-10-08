# SGC (Screen Graphics Controller) 仕様

## 1. 位置付け

SGCはスプライト属性を読み、VRAM上の画像を2Dラスタライズするとともに、CPUがMMIO経由で投入するコマンドFIFOから2D図形を描画するグラフィックス供給元である。出力先はVDPの `GP0`～`GP7` のうち1つを選ぶ。SGC内部の描画順は選択GP内に限られ、GP間の前後関係はVDPのGP番号（小さい番号が奥）で決まる。

SGCのFIFOコマンドは線分、塗りつぶし矩形、塗りつぶし三角形、Unifont glyph描画などの基本2Dプリミティブを扱う。CPUはコマンドをMMIOのFIFO DATAレジスタへ書き込み、SGCが順番に処理する。FIFOはコマンドの一部を分断して保持できるが、不完全なコマンドは必要ワードが揃うまで実行しない。

## 2. 初版機能

- VRAM内Sprite Attribute Table (SAT)からスプライト属性を取得し、4bpp indexed patternを64色CLUTでRGBへ変換する。
- X/Y位置、幅/高さ、H/V flip、整数拡大率、透明インデックス、enableを扱う。
- MMIO FIFOから線分、塗りつぶし矩形、塗りつぶし三角形、Unifont glyphを描画する。
- プリミティブの座標を画面領域でクリップし、指定GPへ画素を供給する。
- スプライト内priority/index順と、プリミティブFIFO順を維持する。

初版では任意角度回転、半透明個別指定、衝突判定、スキャンラインsprite上限、BitBltを実装しない。BitBltは別仕様とする。

## 3. 制約と画素形式

| 項目 | 初版仕様 |
|---|---|
| 表示領域 | 320×240 |
| 最大登録数 | 256 sprites |
| SAT entry | 16 bytes、big-endian |
| sprite位置 | signed 16-bit X/Y、画面左上原点、右/下が正 |
| spriteサイズ | 8-bit幅/高さ、1～64 texels |
| sprite pattern | 4bpp packed indexed pixels、左側pixelを上位nibble |
| sprite palette | 6-bit CLUT index。palette base + texel indexをmodulo 64で参照 |
| sprite透明 | texel index 0を透明 |
| sprite拡大 | X/Y各1～4倍のnearest-neighbor整数拡大 |
| 出力 | SGC全体で選択したGP 1つ |
| FIFO描画色 | 6-bit CLUT index（COLORの下位6 bit） |
| FIFO描画順 | FIFOに投入された順。後から描いた不透明pixelが手前 |

4bppのpattern byte数は `ceil(width * height / 2)`。行ごとにbyte境界へ丸めず、全pixelを連続してpackする。パターン開始アドレスはVRAM相対byte offsetであり、範囲がVRAM外へ出るspriteはそのsprite全体を無効にしてSGCエラーを記録する。

FIFOプリミティブは不透明な単色描画とし、色はSGCの現在のCLUTを参照する。色index 0もFIFO描画では不透明色として扱う。画面外画素は捨てる。LINEの端点は両端を含み、RECTは左上を含み右端・下端を含まない。三角形は頂点を結ぶ辺を含めて塗りつぶす。SCREEN_FILLは描画plane全体を現在色で塗りつぶす。同一共有辺の二重描画は許容する。

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

不正なサイズ、scale、未定義flag bitを持つentryはそのspriteだけを描画せず、INVALID_SPRITEをstickyに立てる。disabled entryは検査対象外。SAT base+count×16がVRAM範囲外ならSGC設定エラーとしてスプライト描画を行わない。

## 5. 表示順

スプライト同士はpriority昇順、次にsprite index昇順で処理し、後に処理した不透明pixelが手前となる。FIFO描画コマンドはスプライト描画より後に処理され、FIFO順に重ねる。実装はフレーム内の描画タイミングを調整してよいが、SGCの不透明な描画結果についてこの論理順を維持する。透明sprite pixelは書き込みを行わない。`GLYPH`はglyph内の0-bitを透明として書き込まない。

## 6. MMIO

SGC MMIO baseは `0x80010000`、sizeは64 KiB。レジスタは32-bit big-endian、4-byte aligned。byte accessはレジスタ内の対応byte laneに反映する。FIFO DATAへのbyte/halfword書込みは各laneをword latchへ蓄積し、4 laneが揃った時点で1 wordを上位byteからFIFOへ投入する。未定義領域は0を読み、書込みを無視する。

| Offset | Register | Access | 定義 |
|---:|---|:---:|---|
| `0x0000` | ID | R | `0x53474331` ("SGC1") |
| `0x0004` | CONTROL | RW | bit0 enable、bit1 soft reset pulse |
| `0x0008` | STATUS | R/W1C | bit0 BUSY、bit1 INVALID_CONFIG、bit2 INVALID_SPRITE、bit3 FIFO_FULL、bit4 FIFO_ERROR |
| `0x0010` | SAT_BASE | RW | VRAM相対byte offset、16-byte aligned |
| `0x0014` | SPRITE_COUNT | RW | 0～256 |
| `0x0018` | OUTPUT_GP | RW | 0～7 |
| `0x0020` | FIFO_DATA | W | FIFOへ32-bit wordを投入。空きがない場合は投入せずFIFO_FULLを記録 |
| `0x0024` | FIFO_STATUS | R | bit31:16 FIFO内word数、bit15:0 空きword数 |
| `0x0028` | FIFO_CONTROL | RW | bit0 FIFO clear pulse |

FIFOは最大256 wordを保持する。FIFO_DATAへの書込みは、空きがない場合にバスを待たせず、そのwordを破棄してFIFO_FULLをstickyに記録する。SGCは各VDP `tick`でFIFO内の完全なコマンドを投入順に消費し、CONTROLのenable=0では内容を保持する。STATUSのBUSYはenable中にFIFOにwordが残っている場合に立つ。soft resetまたはFIFO clearはFIFOと未完コマンドを破棄する。FIFO clear時にFIFO wordまたは書込み途中のbyte laneが残っていればFIFO_ERRORを記録する。FIFO clearは既に描画したコマンド画素を消去しない。soft resetはFIFO、未完word latch、描画済みコマンド画素を消去する。

コマンド画素はSGCの透明320×240 planeに保持され、後続コマンドが不透明画素を上書きする。各frameでspriteを再ラスタライズしてからこのplaneを重ねる。SGC disabled中はFIFOを消費せず、描画planeも出力しない。

RESET後はSGC disabled、sprite count=0、SAT base=0、output GP=1、FIFO空、status clear、描画plane透明。reserved control bitは0を書き、1を書いた場合はINVALID_CONFIGを記録する。設定値は即時反映し、frame latch/double bufferingは初版に含めない。

## 7. FIFOコマンド形式

各コマンドは32-bit word列で表し、すべてbig-endianでFIFOへ投入する。wordのbit31:24がopcode、bit23:0がopcode固有データである。SET_COLORのbit5:0を除くbit23:6と、他の既知opcode wordのbit23:0は0とする。整数座標は符号付き16-bitで、320×240の座標系を使う。座標値は画面外を指定でき、描画時にclipする。

| Opcode | コマンド | Word列 (opcode wordを含む) |
|---:|---|---|
| `0x01` | SET_COLOR | 1 word。bit5:0がCLUT index、上位の予約bitは0 |
| `0x10` | LINE | 5 words。続く4 wordは始点X、始点Y、終点X、終点Y |
| `0x11` | RECT | 5 words。続く4 wordは左上X、左上Y、右下排他的X、右下排他的Y |
| `0x12` | TRIANGLE | 7 words。続く6 wordは頂点1～3それぞれのX,Y |
| `0x13` | GLYPH | 4 words。続く3 wordはUnicode BMP code unit、X、Y |
| `0x14` | SCREEN_FILL | 1 word。320×240の描画plane全体を現在のCLUT色で塗りつぶす |
| `0xFF` | NOP | 1 word。予約・整列用 |

`GLYPH`の第2 wordはBMP code unitを格納し、bit31:16を0とする。第3/第4 wordはglyph左上pixelのsigned 16-bit X/Yで、各wordのbit31:16を0とする。glyph rowはVDP PCGと同じ読み取り専用GNU Unifont BMP CHR ROMを参照する（配置とfallbackはVDP仕様を参照）。現在の`SET_COLOR`色を前景色として使い、0-bitは透明、1-bitは前景色で描く。画面外pixelはclipする。8×16 glyphは16×16 slotの左側8 pixelを使う。未収録code unitとsurrogateはU+FFFD glyphを使う。描画は既存spriteの後、FIFO順に行う。

座標payload wordはbit31:16を0、bit15:0をsigned座標とする。三角形は各整数座標位置をsampleし、辺を含めて塗りつぶす。RECTで右下座標が左上以下の場合、または三角形の全頂点が一直線上の場合は何も描かず、FIFO_ERRORを記録する。未定義opcode、予約bit違反、コマンド途中でのFIFO clearはFIFO_ERRORをstickyに記録する。既知opcodeの不正コマンドはそのopcodeの宣言word数を消費し、後続の解釈を継続する。長さを定義していないopcodeはheader 1 wordのみを消費する。

FIFOにコマンド途中までしか届いていない場合、その先頭wordと後続wordを保持し、完全なコマンドになるまで待つ。コマンド投入はSET_COLORを含めFIFO順に処理される。描画コマンドは単一の現在色を使い、出力GPはOUTPUT_GP設定に従う。LINEは両端を含む整数 Bresenham 線分、RECTは左上を含み右端・下端を含まない矩形、SCREEN_FILLは全描画planeの不透明色塗りつぶしとする。

## 8. VRAM共有と同期

SGCはVDPと同一4 MiB VRAMを共有する。SAT/pattern readはVDP VRAMの相対offsetを用いる。CPUが描画中に属性やpatternを書き換えた場合、次に評価する画素から新しい値を参照する。エミュレータはフレーム取得時にスプライト一覧を読む実装でもよいが、その場合は変更が次フレームから反映される差を実装上の制約として明記する。

## 9. 初版に含めない機能

任意角度回転、fractional scale、sprite collision、line limit/flicker、per-sprite alpha/blending mode、複数GPへの同時出力、BitBltは予約または別仕様とする。将来拡張でも既存entryの予約byte/bitを再利用する場合は、feature/version識別を追加してから有効化する。
