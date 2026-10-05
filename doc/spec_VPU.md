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
| 入力 | 32-bit address space上のコマンド列、頂点データ、テクスチャ（将来拡張） |
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

初版で描画可能なプリミティブは三角形のみとする。コマンドの頂点数に従い、独立三角形（3頂点ずつ）またはtriangle stripを指定できる。stripでは各三角形の向きを交互に反転する。線、点、四角形、テクスチャマッピングは予約扱いとする。

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
- 点光源、フォグ、テクスチャ、アルファブレンディングは初版対象外。

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
| `0x00` | END | payloadなし。FIFO投入バッチの終端 |
| `0x7F` | NOP | 任意 |

頂点アドレスはシステム物理アドレスとする。頂点数・インデックス数は三角形構成可能な数でなければならない。頂点フォーマットはPOSITION (3×f32) 必須、COLOR (RGBA8または4×f32)、NORMAL (3×f32) 任意属性の組み合わせとする。属性はこの順で配置し、COLORはRGBA8を既定とする。インデックスはunsigned 16-bitで、ビッグエンディアンとする。SET_STATEの状態IDは、`0`=シェーディング方式 (0 Gouraud, 1 Phong)、`1`=ライティング有効、`2`=カリング方式 (0 無効, 1 背面除去)、`3`=Zテスト有効、`4`=Z比較関数、`5`=Z書き込み有効、`6`=RGBA書き込みマスク (bit0 R, bit1 G, bit2 B, bit3 A) とする。これらの状態は後続の描画コマンドに適用する。

SET_STATEはpayload 2ワード（状態ID、値）、SET_MATRIXは17ワード（行列ID、16要素を行優先で格納）、SET_TARGETは6ワード（カラー基底、Z基底、幅、高さ、カラーstride、Z stride）、CLEARは3ワード（flags bit0 color clear / bit1 Z clear、RGBA8888、depth）とする。DRAW_TRIANGLESは4ワード、DRAW_INDEXEDは5ワードで、各々のpayload順は表に記載した順とする。vertex format wordはbit0=COLOR有効、bit1=COLORを4×f32形式、bit2=NORMAL有効とする。位置と法線はbinary32、RGBA8カラーは1ワード内の上位バイトからR,G,B,Aとする。ENDはpayloadなしで、FIFOの1バッチを完了させる。DMAでは指定バイト長末尾がバッチ終端となり、ENDを含めてもよい。

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

テクスチャマッピング、透過・ブレンディング、フォグ、ポイント/ライン描画、シェーダープログラム、マルチサンプル、複数同時カラーターゲット、非同期フェンスは初版では規定しない。opcodeおよび状態IDは将来拡張用に予約する。
