# VPU (Vector Processing Unit) 仕様

## 1. 概要

VPUは、CPT32の3Dグラフィックス処理を行う固定機能パイプライン型の描画デバイスである。CPUは頂点・行列・描画状態をメモリに用意し、コマンドFIFOへの直接投入、またはメインRAM上のコマンド列をDMA実行させる。VPUは三角形プリミティブを座標変換、クリッピング、ラスタライズし、カラー結果とZバッファを生成する。カラー結果はVDPの選択したGPへの描画供給元であり、最終表示画素への直接合成は行わない。

本仕様はVPUのソフトウェア可視動作を定義する。実装内部の並列度やクロック数は規定しない。数値形式、コマンド形式、初期描画機能をここで固定し、未定義の拡張機能は予約扱いとする。

## 1.1 エミュレータ実装方針

エミュレータ上のVPUは、CPUやDMAから受け取ったコマンド列を順番に解釈して処理するインタプリタ方式で実装する。描画処理のバックエンドには`wgpu`を使用し、コマンドの状態変更、頂点処理、描画、同期をこの仕様で定義したVPUの動作に対応付ける。`wgpu`はエミュレータ内部の実装手段であり、ゲストソフトウェアから見える動作や数値規則は本仕様に従う。したがって、ホストGPUや`wgpu`のAPIをゲストから直接利用することはできず、バックエンドを変更してもコマンド仕様および描画結果の意味は変わらない。

## 2. システム上の位置付け

| 項目 | 仕様 |
|---|---|
| MMIO領域 | `0x80030000`–`0x8003FFFF` (64 KiB) |
| 入力 | 32-bit address space上のコマンド列、頂点データ、FIFO登録テクスチャ |
| 描画先 | VRAM (`0x10000000`–`0x103FFFFF`)。カラー/ZターゲットはVRAM内。 |
| 標準画面 | 320×240 pixels |
| コマンド投入 | MMIO FIFO またはメインRAMからのDMA実行 |
| バイト順 | 32-bitレジスタおよびコマンドはビッグエンディアン |

VPUのカラーターゲットとZターゲットはVPUコマンドで指定する。出力GPはVPUの表示状態で指定する。VDPはVPUのカラー出力をそのGPの他の供給元と合成し、GP番号に従い最終合成する。GP間の順序、alpha合成、表示タイミングは[VDP仕様](spec_VDP.md)に従う。VPUは表示タイミングを制御しない。

## 3. 固定パイプライン

処理順序は次の通りとする。

1. 頂点属性の読み込み
2. モデル、ビュー、射影行列による座標変換
3. ビュー空間でのライティング（Phong時）
4. クリッピング
5. 透視除算およびビューポート変換
6. 背面カリング（設定時）
7. 三角形ラスタライズと深度補間
8. Zテスト
9. シェーディング（Gouraud補間、またはPhong per-pixel）
10. カラー書き込みと、設定時のZ書き込み

### 3.1 座標と数値形式

- 行列は4×4、32-bit IEEE 754 binary32浮動小数点数、行優先メモリ配置とする。
- 頂点位置はモデル座標の `(x,y,z,1)`。法線は `(x,y,z)` とする。
- 変換は列ベクトル規約 `clip = Projection × View × Model × position` とする。
- NDC可視範囲は `-w ≤ x ≤ w`, `-w ≤ y ≤ w`, `0 ≤ z ≤ w` とする。透視除算後の深度範囲は `[0,1]`。
- ビューポートの左上を `(x,y)`、幅・高さを `(width,height)` とし、画面Yは下向きに増加する。
- 画面外へはみ出す三角形をクリップする。頂点ごとの単純な棄却は行わない。

### 3.2 プリミティブ

初版で描画可能なプリミティブは三角形のみとする。実装済みFIFOコマンドでは独立した三角形を描画できる。線、点、四角形は予約扱いとする。テクスチャ付き三角形は6.6節で規定する。

## 4. シェーディングと深度

### 4.1 Gouraud

頂点ごとに ambient + diffuse + specular の固定機能ライティングを計算し、カラーを三角形内で補間する。ライティング無効時は頂点カラーをそのまま使用する。補間は透視補正付きとする。

### 4.2 Phong

法線および位置を透視補正付きでピクセルごとに補間し、ピクセル単位で固定機能ライティングを計算する。Phongモードでは入力法線を必須とする。法線は補間後に正規化する。

### 4.3 固定機能ライティング

- 最大4個の方向光源および1個の環境光をサポートする。
- 各光源は有効フラグ、方向、RGB色、ambient/diffuse/specular係数を持つ。
- 材質はambient/diffuse/specular RGB係数およびshininess値を持つ。
- RGB値と係数は有限のbinary32値として扱い、最終カラーを0～1にクランプして8-bit RGBAへ変換する。
- 点光源、フォグ、アルファブレンディングは初版対象外。

### 4.4 Zバッファ

- Z値はVRAM内の32-bit binary32配列とする。1ピクセルにつき4バイト。
- Z比較関数は `NEVER`, `LESS`, `LEQUAL`, `EQUAL`, `GEQUAL`, `GREATER`, `ALWAYS` をサポートする。
- ZテストとZ書き込みは独立して有効化できる。標準状態はテスト有効、`LESS`、書き込み有効。
- `CLEAR`コマンドで指定した深度値を範囲内の全ピクセルへ書く。標準クリア値は`1.0`。

## 5. VRAMターゲット

描画ターゲットはVRAM上に配置し、ベースアドレス、幅、高さ、行strideを設定する。初版のカラー形式はRGBA8888、Z形式は32-bit floatのみとする。カラーターゲットのRGBAはstraight alphaとし、VDP GP供給元として解釈する。A=0は透明である。`stride`はバイト単位で、カラーターゲットでは4以上かつ4の倍数、Zターゲットでは4の倍数とする。アドレス範囲がVRAM外に出る設定はエラーとする。

```text
pixel_address = base + y * stride + x * 4
```

各ピクセルのバイト配置はメモリ上でR, G, B, Aの順とする。フレームバッファとZバッファの領域は重複させてはならない。重複または不正なターゲット設定では描画を行わず、エラーステータスを立てる。

## 6. コマンド投入

コマンドストリームは32-bitワード列で、すべてビッグエンディアンとする。各コマンドは先頭ワードの上位8-bit opcode、下位24-bit payload word数で長さを示し、その後にpayloadを置く。長さには先頭ワードを含めない。未知opcode、長さ不足、範囲外メモリアクセスはコマンドエラーとし、そのコマンドを破棄する。エラー後は次のコマンド境界から処理を継続する。

### 6.1 FIFO方式

CPUはFIFO_DATAレジスタへコマンドワードを順番に書き込む。FIFO満杯時の書き込みは受理せず、OVERFLOWを記録する。STATUSで空きワード数を確認できる。FIFOは書き込み順に実行する。

### 6.2 DMA方式

CPUはメインRAM上のコマンド列の開始アドレスとバイト長を設定し、DMA_STARTへ書き込む。アドレスと長さは4バイト境界でなければならない。実行中の再STARTはBUSYエラーとする。コマンド列末尾まで実行後にDONEを立て、完了割り込みを発生させる（割り込み有効時）。DMAはVRAMおよびMMIOをソースとして参照できない。

FIFO入力とDMA実行は同時に開始できない。BUSY中の異なる投入経路からの書き込みは受理せず、BUSYエラーとする。

### 6.3 初版コマンド

| Opcode | 名称 | Payload |
|---:|---|---|
| `0x01` | SET_STATE | 描画状態IDと値。後述の状態項目を更新 |
| `0x02` | SET_MATRIX | matrix ID (MODEL/VIEW/PROJECTION) と16個のbinary32値 |
| `0x03` | SET_TARGET | color base, Z base, width, height, color stride, Z stride |
| `0x04` | CLEAR | flags, RGBA8888 color, binary32 depth |
| `0x10` | DRAW_TRIANGLES | vertex address, vertex count, topology, vertex format |
| `0x11` | DRAW_INDEXED | vertex address, index address, index count, topology, vertex format |
| `0x12` | DRAW_FLAT_TRIANGLE | three screen-space XYZ vertices, packed RGB, diffuse intensity |
| `0x14` | DRAW_TL_TRIANGLE | 3 vertices, each POSITION(float3) and packed RGBA8 color |
| `0x15` | SET_LIGHT | light index, enabled, ambient/diffuse/specular/emission RGB, position XYZ |
| `0x16` | DRAW_TL_LIT_TRIANGLE | 3 vertices, each POSITION(float3), NORMAL(float3), packed RGBA8 color |
| `0x17` | TEXTURE_UPLOAD | texture ID, width, height, packed RGBA4444 texels |
| `0x18` | DRAW_TEXTURED_TRIANGLE | texture ID, shading mode, 3 vertices with POSITION(float3), UV(float2), packed RGBA8 color |
| `0x00` | END | payloadなし。FIFO投入バッチの終端 |
| `0x7F` | NOP | 任意 |

頂点アドレスはシステム物理アドレスとする。頂点数・インデックス数は三角形構成可能な数でなければならない。頂点フォーマットはPOSITION (3×f32) 必須、COLOR (RGBA8または4×f32)、NORMAL (3×f32) 任意属性の組み合わせとする。属性はこの順で配置し、COLORはRGBA8を既定とする。インデックスはunsigned 16-bitで、ビッグエンディアンとする。SET_STATEの状態IDは、`0`=シェーディング方式 (0 Gouraud, 1 Phong)、`1`=ライティング有効、`2`=カリング方式 (0 無効, 1 背面除去)、`3`=Zテスト有効、`4`=Z比較関数、`5`=Z書き込み有効、`6`=RGBA書き込みマスク (bit0 R, bit1 G, bit2 B, bit3 A) とする。これらの状態は後続の描画コマンドに適用する。

SET_STATEはpayload 2ワード（状態ID、値）、SET_MATRIXは17ワード（行列ID、16要素を行優先で格納）、SET_TARGETは6ワード（カラー基底、Z基底、幅、高さ、カラーstride、Z stride）、CLEARは3ワード（flags bit0 color clear / bit1 Z clear、RGBA8888、depth）とする。DRAW_TRIANGLESは4ワード、DRAW_INDEXEDは5ワードで、各々のpayload順は表に記載した順とする。vertex format wordはbit0=COLOR有効、bit1=COLORを4×f32形式、bit2=NORMAL有効とする。位置と法線はbinary32、RGBA8カラーは1ワード内の上位バイトからR,G,B,Aとする。ENDはpayloadなしで、FIFOの1バッチを完了させる。DMAでは指定バイト長末尾がバッチ終端となり、ENDを含めてもよい。

### 6.4 軽量フラット三角形コマンド

`DRAW_FLAT_TRIANGLE` (`0x12`) は、ソフトウェアで変換・投影済みの頂点を送る最小構成の描画コマンドである。payloadは11ワードで、`x0,y0,z0,x1,y1,z1,x2,y2,z2,RGB,intensity` の順とする。頂点座標と深度は符号付き整数、`RGB`は下位24-bitの`0xRRGGBB`、`intensity`は0～255とする。各画素の色は三頂点で一定とし、各RGB成分に`intensity / 255`を乗算する。深度は三角形内で線形補間し、値が小さい画素を手前としてZテスト・更新する。テクスチャ、透視補正、クリッピング、カリングは行わず、画面範囲との交差部分のみラスタライズする。このコマンドはFIFO経由のデモ向けであり、アドレス指定頂点形式とは独立している。

### 6.5 最小変換・クリッピング・シェーディング経路

エミュレータの最小T&L経路では、`SET_MATRIX` (`0x02`) でMODEL、VIEW、PROJECTION行列をそれぞれ設定し、`DRAW_TL_TRIANGLE` (`0x14`) で未変換頂点を投入する。行列はIEEE 754 binary32、行優先で、変換は列ベクトル規約 `clip = Projection × View × Model × position` とする。行列IDは0=MODEL、1=VIEW、2=PROJECTION。

`SET_STATE` (`0x01`) のpayloadは状態IDと値の2語である。状態ID 0はシェーディング方式（0=flat、1=Gouraud）、状態ID 1は固定機能ライティング（0=無効、1=有効）を選ぶ。flatは三角形の一定色を使い、Gouraudは頂点色を透視補正付きで補間する。`DRAW_TL_TRIANGLE` (`0x14`) のpayloadは12語で、各頂点につき位置x/y/zのbinary32を3語、その後にRGBA8を上位バイトから格納した1語を並べる。この旧コマンドはライティングを行わない。

ライトは最大8個で、`SET_LIGHT` (`0x15`) のpayloadは17語である。語順は `light_index, enabled, ambient_r, ambient_g, ambient_b, diffuse_r, diffuse_g, diffuse_b, specular_r, specular_g, specular_b, emission_r, emission_g, emission_b, position_x, position_y, position_z`。色成分と位置はIEEE 754 binary32、enabledは0または1、indexは0～7とする。ライト位置はVIEW座標系で指定する。頂点カラーがマテリアルの拡散色とアルファ値になる。Ambientは頂点カラーとの積、DiffuseはLambert項、Specularはshininess 16固定のBlinn-Phong項として計算し、Emissionはライトごとの一定RGB加算として扱う。各ライトの寄与を合計して画素出力時に0～1へクランプする。距離減衰やスポットライトはない。

`DRAW_TL_LIT_TRIANGLE` (`0x16`) は法線付きT&L三角形で、payloadは21語、各頂点につき `POSITION.x/y/z`、`NORMAL.x/y/z` のbinary32各3語とRGBA8 1語をこの順で格納する。位置と法線はMODEL座標系で入力し、法線はVIEW×MODELの逆転置3×3行列で変換して正規化する。ライティングが無効なら頂点カラーをそのまま使う。有効時、Gouraudはライティング後の頂点色を補間する。flatは3頂点法線の平均と第0頂点のVIEW位置で1回照明計算し、三角形全体に同じ色を使う。隣接三角形で同じflat照明を得るには、第0頂点と法線を共通にする。法線付きコマンドには非特異なVIEW×MODEL上3×3行列が必要となる。

### 6.6 テクスチャ登録とテクスチャ付き三角形

テクスチャ形式はRGBA4444のみとし、幅・高さは各1～1024、行は上から下、画素は左から右に詰める。画素のbyte0は`R<<4 | G`、byte1は`B<<4 | A`。各ニブルを`n * 17`として8-bitへ展開する。テクスチャはIDで参照し、画素データは`TEXTURE_UPLOAD`でVPUにコピーする。

`TEXTURE_UPLOAD` (`0x17`) のpayload長は `3 + ceil(width*height/2)` ワード。語順は `id, width, height, packed_texels...`。各データワードの上位16-bitが先の画素、下位16-bitが次の画素で、各16-bit値は`R:G:B:A`の順に4-bitずつ格納する。画素数が奇数の場合、最後の下位16-bitは無視する。同じIDへの再登録は既存テクスチャを置き換える。最大コマンド長は`3 + 1024*1024/2` payloadワード。寸法、長さが不正な登録はコマンドエラーとし、既存登録を保持する。

`DRAW_TEXTURED_TRIANGLE` (`0x18`) のpayloadは20ワードで、`texture_id, shading_mode`に続き、3頂点それぞれの`POSITION.x/y/z` binary32、`u/v` binary32、RGBA8カラー1ワードの順。shading_modeは0=NONE、1=FLAT、2=GOURAUD。位置・UVは有限値でなければならず、未知ID、不正モード、非有限値はコマンドエラーとなり描画しない。変換・クリッピングは通常のT&L経路に従う。UVは画面上でアフィン補間し、透視補正を行わない。サンプル位置の`floor(u*width), floor(v*height)`を各々テクスチャ境界へクランプする（Clamp-to-edge、最近傍）。

NONEはテクスチャRGBAをそのまま出力する。FLATはコマンド頂点0のRGBAを三角形全体に用い、GOURAUDは頂点RGBAを画面空間で補間する。SET_STATEでライティングが有効なら、三角形の幾何法線を求め、FLATは頂点0のVIEW位置、GOURAUDは各頂点のVIEW位置で既存固定機能ライトを評価してから色を補間する。NONEではライティングを適用しない。FLAT/GOURAUDはテクスチャと頂点色の各チャンネルの積を計算し、最近傍の整数へ丸めてRGBA8888として出力する。ブレンディングは行わない。

T&L経路はクリップ空間で6面（`-w≤x≤w`, `-w≤y≤w`, `0≤z≤w`）に対してポリゴンをクリップし、生成頂点の色・UVも補間する。残ったポリゴンは画面座標へ変換され、深度テスト付きで塗りつぶす。画面サイズは320×240固定。

## 7. MMIOレジスタ

レジスタはベース `0x80030000` からのオフセットで示す。レジスタは32-bit、4バイト境界とし、バイト単位アクセスは許可するが、FIFO_DATAへのコマンド投入は32-bit書き込みのみ有効。未定義領域は読み出し`0`、書き込み無視とする。

| Offset | Register | Access | 説明 |
|---:|---|:---:|---|
| `0x0000` | ID | R | VPU識別値 `0x56505531` ("VPU1") |
| `0x0004` | STATUS | R/W1C | bit0 BUSY, bit1 FIFO_FULL, bit2 DONE, bit3 COMMAND_ERROR, bit4 TARGET_ERROR, bit5 FIFO_OVERFLOW |
| `0x0008` | CONTROL | RW | bit0 IRQ_ENABLE, bit1 FIFO_RESET (write-1 pulse), bit2 SOFT_RESET (write-1 pulse) |
| `0x000C` | FIFO_LEVEL | R | FIFO使用中ワード数 |
| `0x0010` | FIFO_CAPACITY | R | FIFO容量（ワード数、実装固定値） |
| `0x0014` | OUTPUT_GP | RW | VDP出力先GP (0～7、reset値7)。無効値は無視してCOMMAND_ERRORを立てる |
| `0x0020` | DMA_ADDRESS | RW | コマンド列開始アドレス |
| `0x0024` | DMA_LENGTH | RW | コマンド列長（バイト） |
| `0x0028` | DMA_START | W | bit0に1を書いてDMA実行開始 |
| `0x002C` | FIFO_DATA | W | コマンドFIFOへ1ワード投入 |
| `0x0030` | IRQ_ACK | W | bit0に1を書いてDONEをクリア |

DMAまたはFIFOの全コマンド処理完了時にDONEを立てる。IRQ_ENABLE時はVPU完了IRQをIRQCへ通知する。IRQ番号はシステム割り込み表のデバイスイベント枠（IRQ5）を使用する。

## 8. エラー、同期、リセット

- STATUSの各エラービットとDONEはwrite-1-to-clearとする。BUSYとFIFO_FULLは状態から導出する。
- 描画コマンドは投入順に実行され、後続コマンドは先行する状態変更・描画の完了後に評価される。
- CPUはBUSY解除またはDONEをポーリングして完了を確認できる。
- SOFT_RESETはFIFO、実行状態、エラーフラグを初期化し、VRAM内容は変更しない。
- リセット後は標準ビューポート(320×240)、Gouraud、ライティング無効、背面カリング無効、ZテストLESS有効、Z書き込み有効、RGBA書き込み有効とする。描画ターゲットのベースアドレスは未設定であり、設定前の描画はTARGET_ERRORとなる。

## 9. 初版の範囲外

透過・ブレンディング、フォグ、ポイント/ライン描画、シェーダープログラム、ミップマップ、マルチサンプル、複数同時カラーターゲット、非同期フェンスは初版では規定しない。テクスチャ解放コマンドも未実装であり、再登録またはVPUリセットで破棄する。
