# VPU の使い方チュートリアル

このページでは、sol から VPU の FIFO に描画コマンドを送り、塗りつぶし三角形を描く手順を説明します。まず既存の [3D キューブサンプル](../3d_cube.sol) と同じ軽量経路を使います。

## できることと実装範囲

現在のエミュレータで実際に使える描画コマンドは、画面クリア (`CLEAR`, `0x04`) とフラットシェーディング三角形 (`DRAW_FLAT_TRIANGLE`, `0x12`) です。コマンドは FIFO 経由で送ります。キューブの回転、座標変換、透視投影は sol 側で行い、VPU は画面座標の三角形を塗りつぶします。

三角形は各画素で一定の RGB 色を使います。コマンドに渡した明度係数を VPU が RGB に乗算するため、面ごとに色や明るさを変えられます。深度は三角形内で補間し、値が小さいものを手前として扱います。テクスチャ、クリッピング、背面カリング、Gouraud/Phong ライティングは、この軽量経路では使えません。

より広いコマンド形式と将来の仕様は [VPU 仕様書](spec_VPU.md)を参照してください。仕様書にある行列、アドレス指定頂点、DMA などの機能の一部は、現時点のエミュレータではまだ実装されていません。

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

実用的な回転キューブ全体は [3d_cube.sol](../3d_cube.sol) にあります。そこでは8頂点を sol で回転・投影し、投影座標と深度を RAM に保存してから、6面を FIFO コマンドで送っています。描画ループの各フレームで `clear_frame`、頂点変換、各面の描画、角度更新を行います。

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
