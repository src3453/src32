# VPU Terrain Flight in C

[`vpu_terrain_flight.sol`](../../vpu_terrain_flight.sol) のC移植です。LLVM/SRC32ツールチェーンとC SDKを使い、VPUで空・地形、SGCで操作ガイドを表示します。

地形はワールド座標から生成する2段階の周期的な格子ノイズです。smoothstepで補間し、標高と斜面による色、遠景の霞をGouraud補間します。フレーム番号で高さを変更しないため、元の84フレーム周期の巻き戻りや14フレーム境界での地形の飛びを避けます。12×18セルの共有頂点をフレームごとに一度計算し、432三角形を描画します。遠い行は幅を広げ、画面の左右を覆います。

## 操作

エミュレータのゲーム画面にフォーカスして操作します。押し続けている間、各描画更新で操作します。

| キー | 操作 |
|---|---|
| ← / → | 左右旋回 |
| ↑ / ↓ | 上昇 / 下降（高度40～160） |
| W / S | 機首を上げる / 下げる |
| A / D | 左右へ横移動 |
| Q / E | 減速 / 加速（0～8、初期速度2） |
| Space | 前進・横移動の一時停止 / 再開。視点・高度は操作可能 |
| R | 初期位置・姿勢・速度へ戻り、自動飛行を再開 |

PeCのUSB HIDイベントをpeekして明示的にpopし、押下と解放を保持します。Space/Rは押下エッジだけで動作します。地形の高さは最大約30で、最低高度40でも地面の上を飛びます。ワールドは24576単位で周期的につながります。

## ビルド・実行

リポジトリルートで実行します。Pythonの依存関係とClangは[ツールチェーンのREADME](../../tools/llvm-src32/README.md)を参照してください。

```powershell
New-Item -ItemType Directory -Force target/terrain-c/tmp | Out-Null
$env:TMP = (Resolve-Path target/terrain-c/tmp).Path
$env:TEMP = $env:TMP
python tools/llvm-src32/driver.py examples/vpu-terrain-flight-c/main.c sdk/cpt32/src/vpu.c sdk/cpt32/src/irqc.c -I sdk/cpt32/include -O2 --clang "C:/Program Files/LLVM/bin/clang.exe" -o target/terrain-c/vpu_terrain_flight.wax
cargo run --release --bin cpt32 -- target/terrain-c/vpu_terrain_flight.wax
```

CMake/Ninjaでもビルドできます（Clang/Python/Ninjaのパスは環境に合わせて指定）。

```powershell
cmake -S examples/vpu-terrain-flight-c -B target/terrain-c/cmake -G Ninja -DSRC32_CLANG="C:/Program Files/LLVM/bin/clang.exe"
cmake --build target/terrain-c/cmake --target vpu_terrain_flight
```

CMakeの出力は `target/terrain-c/cmake/vpu_terrain_flight.wax` です。

## 検証

CLIの手順でWAXを作成した後、実ゲストの描画・入力を検証します。BIOSやディスクを読み込まない専用のBusを使います。

```powershell
cargo test --release --test vpu_terrain_flight -- --ignored --nocapture
```

Cached/Pipelineで、空だけでなく緑の地形が表示されること、画面に未描画領域がないこと、VPUにコマンドエラーがないこと、一時停止、旋回の長押しと解放、飛行再開、リセットの再現性を検査します。画面は `target/terrain-c/terrain.ppm` に保存します。通常のRustテストでは外部Clangビルドを要求しないよう、このテストは明示実行です。

整数Q10で座標・行列を計算し、FIFOへ送る時点でbinary32のビット列へ変換します。Cの浮動小数点命令は使いません。VSyncはホストのIRQ0とSDKのdispatcherを使います。描画更新がホスト1フレームの予算を超える場合、更新頻度は下がります。

空は深度1より少し手前、地形より遠くに描きます。現在のVPU FIFOのSET_STATEはID 0/1のみ対応し、仕様にあるZテストやカリングの切替は使いません。CLEARには仕様上の遠深度1.0を渡します（現実装は指定深度にかかわらず1.0へクリアします）。
