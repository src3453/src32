# VPU の使い方チュートリアル

このページでは、sol から VPU の FIFO に描画コマンドを送り、行列変換済みの立体を描く手順を説明します。回転キューブのサンプルは [3D キューブ](../3d_cube.sol) を参照してください。

## できることと実装範囲

現在のエミュレータでは、画面クリア (`CLEAR`, `0x04`)、フラット三角形 (`DRAW_FLAT_TRIANGLE`, `0x12`)、変換前頂点の描画 (`DRAW_TL_TRIANGLE`, `0x14`)、ライト設定 (`SET_LIGHT`, `0x15`)、法線付き変換頂点の描画 (`DRAW_TL_LIT_TRIANGLE`, `0x16`)、変換行列設定 (`SET_MATRIX`, `0x02`)、シェーディング方式選択 (`SET_STATE`, `0x01`) を FIFO 経由で使えます。`0x12` は投影済み座標用です。`0x14` と `0x16` は頂点を Model/View/Projection 行列で変換してから描きます。

`0x16` は頂点法線をVIEW×MODELの逆転置行列で変換して照明します。ライトは最大8個まで設定でき、各ライトはAmbient、Diffuse、Specular、Emission、VIEW座標のPositionを持ちます。flatは三角形全体で一定の照明色、Gouraudは頂点ごとの照明色を補間します。T&L経路はクリップ空間の6面をクリップし、深度テストして塗りつぶします。テクスチャと背面カリングはありません。`0x12` の明度係数付き画素描画経路も引き続き使えます。

より広いコマンド形式と実装範囲は [VPU 仕様書](spec_VPU.md)を参照してください。アドレス指定頂点、インデックス描画、DMA、距離減衰付きライトなどはまだ実装されていません。

## MMIO アドレス

| アドレス | 用途 |
|---|---|
| `0x80000000` | VDP 表示有効レジスタ |
| `0x80000001` | VDP 表示モード。`0` がグラフィックスモード |
| `0x80000003` | VDP 枠色 |
| `0x80030004` | VPU STATUS |
| `0x8003002C` | VPU FIFO_DATA |

通常の VDP レジスタは sol の `stb` で書けます。FIFO_DATA にはコマンド語を 32-bit の `st` で書き込みます。各コマンド語は上位バイトから送ります（ビッグエンディアン）。

## 1. FIFO にコマンド語を送る

まず、32-bit の値を FIFO に書く関数を用意します。

```sol
!const VPU_FIFO 0x8003002C

fn emit (word) :
    word VPU_FIFO st
;
```

コマンドの先頭語は、上位 8-bit が opcode、下位 24-bit が payload の語数です。たとえば `0x1200000B` は opcode `0x12`、payload 11 語を意味します。

## 2. 画面と深度をクリアする

`CLEAR` の payload は flags、RGBA8888、深度値の 3 語です。flags の bit 0 はカラー面、bit 1 は深度面をクリアします。

```sol
fn clear_frame () :
    0x04000003 emit  # CLEAR、payload は3語
    3 emit           # bit0=color, bit1=depth
    0 emit           # RGBA = 透明な黒
    0 emit           # 現実装では深度値を参照せず、最大値へ初期化
;
```

カラー面を透明にしてから描くと、VDP の他の描画面を隠さずに VPU の画素を重ねられます。深度クリアを省くと、前のフレームの深度が残り、新しい三角形が隠れることがあります。

## 3. フラットシェーディング三角形を描く

`DRAW_FLAT_TRIANGLE` の payload は 11 語です。

| 順 | 値 |
|---:|---|
| 1–3 | 頂点0の `x, y, depth` |
| 4–6 | 頂点1の `x, y, depth` |
| 7–9 | 頂点2の `x, y, depth` |
| 10 | `0x00RRGGBB` の RGB 色 |
| 11 | 明度係数 `0`～`255` |

座標は画面ピクセル単位の符号付き整数です。左上が `(0,0)`、画面サイズは 320×240 です。深度も整数で、小さい値ほど手前です。明度 `255` は指定色そのまま、`128` は各 RGB 成分をおよそ半分にします。

```sol
fn triangle (x0 y0 z0 x1 y1 z1 x2 y2 z2 rgb intensity) :
    0x1200000B emit
    x0 emit  y0 emit  z0 emit
    x1 emit  y1 emit  z1 emit
    x2 emit  y2 emit  z2 emit
    rgb emit
    intensity emit
;
```

たとえば、画面中央に赤い三角形を描きます。

```sol
clear_frame
80 60 100  240 100 100  160 190 100  0x00FF4020 220 triangle
```

三角形の頂点順はどちら向きでも描画されます。現在の軽量経路はカリングしません。画面外の部分は画面境界で描画範囲を制限しますが、幾何学的なクリッピングはしません。

## 4. VDP に VPU 面を表示する

VDP の表示を有効にし、グラフィックスモードを選びます。グラフィックスモードの値は `0` です。

```sol
!const VDP_ENABLE 0x80000000
!const VDP_MODE 0x80000001
!const VDP_BORDER 0x80000003

1 VDP_ENABLE stb
0 VDP_MODE stb
0 VDP_BORDER stb
```

VPU の出力 GP はリセット時に 7 です。描画面と合成する GP を変える場合は VPU の `OUTPUT_GP`（`0x80030014`）を設定します。

## 5. キューブのような立体を描く

立体は三角形に分割し、各面を2枚の三角形として送ります。各面の色と明度を一定にするとフラットシェーディングになります。隠れる面は深度テストで隠します。

```sol
fn face (a b c d rgb intensity) :
    # 四角形 a-b-c-d を2枚の三角形に分ける
    a b c rgb intensity triangle
    a c d rgb intensity triangle
;
```

実用的な回転キューブ全体は [3d_cube.sol](../3d_cube.sol) にあります。sol は固定小数点の三角関数から各フレームの Model 行列を作り、View 行列と透視 Projection 行列を設定します。キューブの頂点はオブジェクト座標のまま FIFO に送り、VPU が座標変換、深度処理、塗りつぶしを行います。各面の頂点色を同じにして、面単位のフラットシェーディングにしています。

## 6. VPU 内で行列変換とクリッピングを行う

`SET_MATRIX` はpayload 17語で、行列IDと16個のbinary32値を送ります。次の例は単位行列を1つ登録する関数です。行列を3つ登録すると `Projection × View × Model × position` の順に適用されます。

```sol
fn set_identity_matrix (id) :
    0x02000011 emit
    id emit
    0x3F800000 emit 0 emit 0 emit 0 emit
    0 emit 0x3F800000 emit 0 emit 0 emit
    0 emit 0 emit 0x3F800000 emit 0 emit
    0 emit 0 emit 0 emit 0x3F800000 emit
;

0 set_identity_matrix # MODEL
1 set_identity_matrix # VIEW
2 set_identity_matrix # PROJECTION
```

`DRAW_TL_TRIANGLE` はpayload 12語です。各頂点を「x, y, z のbinary32ビット列、RGBA8888」の4語で送ります。色のバイト順は上位から R, G, B, A です。シェーディング状態ID 0に値0（flat）または1（Gouraud）を設定します。

```sol
0x01000002 emit # SET_STATE, payload 2語
0 emit           # 状態ID: shading mode
1 emit           # Gouraud

0x1400000C emit # DRAW_TL_TRIANGLE, payload 12語
0xBF000000 emit 0xBF000000 emit 0x3F000000 emit 0xFF0000FF emit
0x3F000000 emit 0xBF000000 emit 0x3F000000 emit 0x00FF00FF emit
0 emit           0x3F000000 emit 0x3F000000 emit 0x0000FFFF emit
```

位置値はbinary32そのものではなく、32-bit語としてのIEEE 754ビット列です。`vpu_tnl_triangle.sol` は単位行列、Gouraud色、画面右端からはみ出す頂点を使い、VPU側の行列処理とクリッピングを実演します。

## 7. 法線とライトでシェーディングする

`SET_LIGHT` (`0x15`) でライトを登録し、`SET_STATE` の状態ID 1を1にしてライティングを有効にします。ライトは最大8個（番号0～7）です。コマンドのpayloadは17語で、色と位置はbinary32のビット列、位置はVIEW座標です。

```sol
0x15000011 emit # SET_LIGHT, payload 17語
0 emit           # ライト番号 0
1 emit           # enabled
0x3DCCCCCD emit 0x3DCCCCCD emit 0x3DCCCCCD emit # Ambient RGB = 0.1
0x3F666666 emit 0x3F666666 emit 0x3F666666 emit # Diffuse RGB = 0.9
0x3E99999A emit 0x3E99999A emit 0x3E99999A emit # Specular RGB = 0.3
0 emit 0 emit 0 emit                         # Emission RGB = 0
0xC2C80000 emit 0x42C80000 emit 0x42A00000 emit # Position = (-100, 100, 80)

0x01000002 emit # SET_STATE
1 emit           # 状態ID: lighting
1 emit           # 有効
```

法線付き三角形 `DRAW_TL_LIT_TRIANGLE` (`0x16`) のpayloadは21語です。各頂点につき位置XYZ、法線XYZをbinary32で送り、その後にRGBA8888を送ります。位置と法線はMODEL座標で指定します。AmbientとDiffuseは頂点色を材質色として計算し、Specularはshininess 16固定、Emissionはライトごとの一定色を加算します。シェーディング状態ID 0を0にするとflat、1にするとGouraudです。

```sol
0x16000015 emit # DRAW_TL_LIT_TRIANGLE, payload 21語
# 頂点0: position (x,y,z), normal (nx,ny,nz), RGBA
0xBF000000 emit 0xBF000000 emit 0x3F000000 emit
0 emit 0 emit 0x3F800000 emit 0xFF8040FF emit
# 頂点1と頂点2も同じ7語形式で続ける
```

無効化するライトは同じ `SET_LIGHT` を送り、enabledを0にします。色の計算式とコマンドの全payloadは [VPU仕様書](spec_VPU.md) を参照してください。

## ビルドと実行

リポジトリのルートから sol を SRC32 アセンブリへ変換し、バイナリにします。

```powershell
python tools/solc/solc.py compile 3d_cube.sol -o 3d_cube.a
python tools/asm/asm.py 3d_cube.a -o 3d_cube.bin
```

ヘッドレス実行器で動作確認する場合は、VPU を含むデバイス構成を使い、ループするデモを実行します。

```powershell
cargo run --bin src32_testbench -- 3d_cube.bin --allow-running
```

`--allow-running` は、フレームループが続くプログラムをサイクル上限まで実行する指定です。実行器は VPU のエラーフラグと画面内に描画画素があることを確認します。

Gouraud補間とクリッピングの三角形サンプルを実行するときは、`3d_cube.sol` と出力名を `vpu_tnl_triangle.sol` と `vpu_tnl_triangle.a` / `vpu_tnl_triangle.bin` に置き換えます。

## 問題が起きたとき

- 画面が空なら、FIFO_DATA への書き込みが `st` になっているか、VDP がグラフィックスモード (`0`) かを確認します。
- 前フレームの図形が残るなら、毎フレームの先頭でカラーと深度の両方をクリアします。
- 三角形が重なる順序がおかしければ、より手前のものに小さい深度を渡します。
- VPU STATUS の bit 3 が立ったら opcode、payload 語数、FIFO への書き込み語数を見直します。`DRAW_FLAT_TRIANGLE` は先頭語 `0x1200000B` に続けてちょうど11語が必要です。
- 画面全体が遅い場合、VPU は CPU が送った三角形をソフトウェアで画素ごとに塗ります。デモでは三角形の数と画面上の大きさを抑えてください。

## 関連資料

- [VPU コマンド・レジスタ仕様](spec_VPU.md)
- [VDP 合成仕様](spec_VDP.md)
- [回転キューブの sol サンプル](../3d_cube.sol)
- [VPU T&L・クリッピング・Gouraud の sol サンプル](../vpu_tnl_triangle.sol)
