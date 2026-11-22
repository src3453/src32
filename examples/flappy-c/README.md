# Flappy（ベアメタル C）

土管のすき間をくぐって飛ぶ、1 ボタンの横スクロールゲームです。GP0 の 256 色ビットマップ（320×240）だけで描画し、音はありません。

- **操作**: Space／↑／W／Enter で羽ばたきます。タイトルで押すと開始、ゲームオーバー後（約 0.5 秒後から）押すとタイトルへ戻ります。
- 土管を 1 本くぐるごとに 1 点です。BEST は電源を切るまで保持します。

## 描画の仕組み

VDP はホストのフレーム開始時（IRQ0 の直前）に GP0 を取り込むので、`irqc_wait_vsync()` の直後に毎フレーム画面全体を描き直し、1 フレーム（800,000 サイクル）以内に描き終えれば、描画途中の画面は表示されません。

DMAC の要求 1 回には設定とポーリングで約 1000 サイクルかかります。文字を 1 ランずつ塗ると 1 フレームに収まらないため、部品は起動時に Main RAM の作業域（`0x00200000` から。Main RAM 4 MiB 以上が必要）へ一度だけ描き、毎フレームは少数の BitBlt（channel 2）で組み立てます。

| 部品 | 方法 |
|---|---|
| 背景（空・雲・街・茂み） | 0〜3 byte ずらした 4 枚を用意し、視差スクロールでも 32-bit beat で 2 回の BitBlt |
| 土管 | 1 行分を source stride 0 で縦に繰り返し、キャップは 42×12 の部品 |
| 地面 | 32 px 周期の帯を画面より 32 px 広く描き、オフセットで転送 |
| 鳥・文字 | カラーキー（index 0 = 透明）の 8-bit BitBlt。文字は内容が変わったときだけ再描画 |

色は SGC の文字色が使う CLUT 0〜63 を避け、64 番以降を使います。描画時間は通常 6〜12 万サイクル、最大でも約 29 万サイクル（得点の文字を描き直すフレーム）です。

## ビルド

CMake と Ninja がある場合:

```powershell
cmake -S examples/flappy-c -B target/flappy-c -G Ninja `
  '-DSRC32_CLANG=C:/Program Files/LLVM/bin/clang.exe' `
  '-DSRC32_PYTHON_EXECUTABLE=C:/path/to/python.exe' `
  '-DCMAKE_MAKE_PROGRAM=C:/path/to/ninja.exe'
cmake --build target/flappy-c
```

driver.py を直接使う場合（リポジトリルートで）:

```powershell
python tools/llvm-src32/driver.py --clang "C:/Program Files/LLVM/bin/clang.exe" -O2 -I sdk/cpt32/include --origin 0 --stack-top 0x00100000 examples/flappy-c/main.c sdk/cpt32/src/dmac.c sdk/cpt32/src/irqc.c -o target/flappy-c/flappy.wax
```

## 実行

BIOS を設定していない（`config.yaml` に `bios` がない）ディレクトリから起動します。

```powershell
& C:/path/to/src32/target/release/cpt32.exe C:/path/to/src32/target/flappy-c/flappy.wax
```

## 検証用ビルドとテスト

- `-DFLAPPY_PROFILE`（`sdk/cpt32/src/print.c` も渡す）: 各フレームの `draw()` のサイクル数を PeC timer 0 で測り、UART に 1 行ずつ出します。
- `-DFLAPPY_AUTOPILOT`: 次の土管のすき間の中央より下に落ちたら自動で羽ばたきます。開始には 1 回キーが必要です。

```powershell
$env:CLANG = 'C:/Program Files/LLVM/bin/clang.exe'
python -m unittest discover -s examples/flappy-c -v
```

タイトル画面、1 回羽ばたいて落ちたときのゲームオーバー画面、オートパイロットで 2 点に達した画面の画素と、全フレームの描画が 1 フレームの半分以内に収まることを確かめます。Clang か `src32_testbench`（`SRC32_TESTBENCH`）がなければ skip します。
