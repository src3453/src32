# 3D Cube in C (LLVM/SRC32)

`main.c` はリポジトリ直下の [`3d_cube.sol`](../../3d_cube.sol) を、独立した `tools/llvm-src32` の Clang/LLVM IR→SRC32 ツールチェーンで移植したゲストプログラムです。`csc` は使いません。

元の操作と描画を維持しています。W/S はカメラ距離、A/D は回転速度、J/K は赤、U/I は緑、N/M は青の照明を調整します。描画はVPU、テキスト表示はSGC、キーボードはPeC、乱数はPeC RNGを使用します。ソースの32-bit浮動小数点値はVPU FIFOへIEEE-754 binary32のビット列として送ります。回転と行列計算はQ10整数で行い、LLVMツールチェーンの対象外である浮動小数点命令に依存しません。

## 必要なツール

- LLVM/Clang 20（この版のIRは、同梱ツールチェーンの `llvmlite 0.50` で検証しています。Windows例: `winget install --id LLVM.LLVM --exact --version 20.1.8`）
- Python 3.11以降と依存パッケージ（リポジトリルートで `python -m pip install -r tools/llvm-src32/requirements.txt ninja`）
- CMake 3.20以降。NinjaはPATHにあるか、下記のCMake引数で指定します。
- 実CPUで起動する場合は `cargo build --bin src32_testbench`

## ビルド

リポジトリルートから実行します。`SRC32_CLANG` は実際のClang 20実行ファイルを指定します。NinjaがPATHにない場合も、Pythonパッケージの実行ファイルを指定できます。

```sh
cmake -S examples/3d-cube-c -B target/cube-c -G Ninja \
  -DSRC32_PYTHON_EXECUTABLE=/path/to/python \
  -DSRC32_CLANG="C:/Program Files/LLVM/bin/clang.exe" \
  -DCMAKE_MAKE_PROGRAM=/path/to/ninja
cmake --build target/cube-c --target three_d_cube
```

出力は `target/cube-c/three_d_cube.wax` です。WAXはoriginとentryを保持するため、raw `.bin` ではなくこの出力を使います。`sdk/cpt32` の表示・乱数・IRQライブラリを `OPTIMIZATION 0` でリンクします。

## 実行

BIOSを設定していない作業ディレクトリから実行します。WAXのentryからRAM上で起動します。

```sh
/absolute/path/src32_testbench /absolute/path/three_d_cube.wax --allow-running --expect-sgc-overlay --expect-gp-noise-background --frames 120
```

GUIで試す場合:

```sh
cargo run --release --bin cpt32 -- target/cube-c/three_d_cube.wax
```

ゲストは[`Cライブラリ`](../../sdk/cpt32/README.md)のIRQ0 dispatcherでVSyncを数え、`irqc_wait_vsync()`で毎フレーム更新します。起動コードが割り込みベクタを登録し、レジスタを保存・復元して`IRET`するため、未登録ベクタへ飛ぶ以前のハングは解消しています。Scalar/Pipelineで120フレーム後もrunning=true、cube・overlay・GP背景検査の通過を確認しています。浮動小数点ランタイムは不要です。
