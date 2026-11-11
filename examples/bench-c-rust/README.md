# C と Rust のベンチマーク（llvm-src32）

同じアルゴリズムの 6 つのカーネルを C（[`c/kernels.c`](c/kernels.c)）と Rust（[`rust/src/lib.rs`](rust/src/lib.rs)）で書き、[llvm-src32](../../tools/llvm-src32/README.md) でそれぞれベアメタルの WAX にして `src32_testbench` で実行し、速度・スタック・コードサイズを比べます。

```sh
python examples/bench-c-rust/run.py                       # C -O2 / Rust opt-level 2 / --cpu pipeline
python examples/bench-c-rust/run.py --c-opt s --rust-opt s --cpu cached --symbols 10 --json out.json
```

必要なもの: Clang、Cargo と `rust-src`（rustc の LLVM メジャー版が llvmlite と同じ。[Rust ガイド](../../sdk/rust/README.md)）、`src32_testbench`（`cargo build --release --bin src32_testbench`）。`--testbench`／`SRC32_TESTBENCH`、`--clang`／`CLANG` で指定できます。ビルド物は `target/bench-c-rust/`（`--work`）に置きます。

## 仕組み

計測は C のハーネス [`harness/harness.c`](harness/harness.c) が行い、**C 版・Rust 版の両方に同じファイルを同じ `-O` でリンク**します（Rust 版は `cargo_src32.py` に `.c` を追加入力として渡す）。違うのはカーネルだけです。カーネルの入口は [`harness/bench.h`](harness/bench.h) の `uint32_t bench_<name>(uint32_t *buffer, uint32_t n)` で、Rust 側は `#[unsafe(no_mangle)] extern "C"` です。

各カーネルは入力を作り直して 2 回実行し、2 回目を報告します。

- **速度**: PeC タイマ 0（free-running、prescaler 0）の 48 MHz システムクロック数。CPU が消費したクロック数で進み、PeC へのアクセスで遅延サイクルも反映されます（[`doc/spec_PeC.md`](../../doc/spec_PeC.md)）。表の値はタイマを続けて 2 回読む分を引いたものです。
- **スタック**: 呼出し前にスタック下方 32 KiB を模様で塗り、終了後に一番下の書き換わった語を探します。カーネルは 4 KiB のフレームの内側から呼ぶので、塗った領域の中で始まります。表の値は空の C 関数（`empty`）の値を引いたものです。
- **コードサイズ**: `--save-temps` の `output.a` を組み立て、トップレベルのラベル間のバイト数を数えます。名前に `bench_<kernel>` を含む関数（C の `bench_sort_partition` など、Rust の `bench_sort` モジュール内の関数）をそのカーネルに数え、残り（ハーネス、`memcpy` などのランタイム、Rust の `core`・panic 処理）を「harness + runtime + core」に数えます。`data` はゼロ初期化を含む静的データ（ハーネスの 32 KiB のバッファを含む）です。
- **正しさ**: 戻り値を `run.py` の Python 実装と比べ、違えば終了コード 1 です。

## カーネル

| 名前 | 内容 | 入力 |
|---|---|---|
| `crc32` | CRC-32（IEEE、`zlib.crc32` と同じ）。256 語の表をスタック上に作る | LCG の上位 8 bit で作った 16 KiB |
| `sieve` | エラトステネスの篩、byte のフラグ。素数の個数 | n = 32768 |
| `sort` | クイックソート（中央要素の Hoare 分割、16 要素未満は挿入ソート、小さい側を再帰）。`sum(a[i] * (i + 1))` | LCG の 2000 語 |
| `matmul` | 24×24 の int32 行列積（wrapping）。`h = h * 31 + C[i]` | LCG の上位 4 bit − 8 |
| `nqueens` | 9-queens のビットマスク再帰探索。解の数 | — |
| `mandel` | Q12 固定小数点の Mandelbrot、48×32、最大 32 回。反復回数の合計 | — |

LCG は `x = x * 1664525 + 1013904223`（seed は crc32 = 1、sort = 2、matmul = 3）。

Rust 版は普通の safe Rust です。スライスの添字（境界検査あり）、イテレータ、`wrapping_*` を使い、`unsafe` は生ポインタからスライスを作るところだけです。アルゴリズムを変えるときは両方の版と `run.py` の `reference()` を同時に直してください。

`run.py` は Rust 版をもう 1 つ、Cargo の feature `unchecked` 付きでビルドします（表の「Rust unchecked」）。カーネル内の添字はすべて `at`／`set`／`swap` を通っており、この feature では `get_unchecked` と `ptr::swap` になります。境界検査の分だけを差し引いた比較用で、安全性はアルゴリズムが範囲内に収まることだけに依存します。入口の `assert!` と `split_at_mut` の検査は残ります。

## 結果の例

C -O2、Rust opt-level 2、`--cpu pipeline`（2026-11-12、Rust 1.96 / llvmlite 0.50）:

| cycles | C | Rust | Rust/C | Rust unchecked | unchecked/C |
|---|--:|--:|--:|--:|--:|
| crc32 | 1,725,270 | 1,463,423 | 0.85 | 1,463,423 | 0.85 |
| sieve | 5,901,211 | 4,626,721 | 0.78 | 4,624,163 | 0.78 |
| sort | 1,784,075 | 2,539,774 | 1.42 | 2,203,550 | 1.24 |
| matmul | 1,219,301 | 1,866,571 | 1.53 | 1,493,851 | 1.23 |
| nqueens | 1,614,680 | 1,614,685 | 1.00 | 1,614,685 | 1.00 |
| mandel | 2,103,842 | 2,104,284 | 1.00 | 2,104,284 | 1.00 |

| bytes | C stack | Rust stack | unchecked stack | C code | Rust code | unchecked code |
|---|--:|--:|--:|--:|--:|--:|
| crc32 | 1,284 | 1,152 | 1,152 | 1,968 | 1,048 | 1,048 |
| sieve | 152 | 140 | 136 | 1,308 | 1,572 | 1,244 |
| sort | 1,900 | 2,716 | 2,360 | 2,692 | 4,492 | 3,356 |
| matmul | 180 | 244 | 200 | 1,364 | 2,024 | 1,560 |
| nqueens | 1,372 | 1,372 | 1,372 | 908 | 928 | 928 |
| mandel | 184 | 192 | 192 | 1,232 | 1,272 | 1,272 |

| image (bytes) | C | Rust | Rust unchecked |
|---|--:|--:|--:|
| code | 19,920 | 29,912 | 27,936 |
| harness + runtime + core | 10,448 | 18,576 | 18,528 |
| data | 32,884 | 34,424 | 34,392 |

- スライスを順に読む `crc32`・`sieve` は Rust のイテレータ（`for &b in data`、`fill`、`sum`）のほうが速い。演算だけの `nqueens`・`mandel` は同じ。
- 添字の多い `sort`・`matmul` は Rust が 1.4〜1.5 倍遅い。境界検査を外すと 1.23〜1.24 倍まで縮み、スタックとカーネルのコードも C に近づくが、差の半分ほどは境界検査以外が原因で残る（未調査）。
- Rust のイメージが大きいのは主に panic 経路です（境界検査の失敗メッセージ用の `core::fmt` の整数表示 約 5 KB、panic handler、`Location` などのデータ）。
- opt-level 0 では Rust の `core` の関数が LTO 後も大量に残り（コード 約 190 KB）、カーネルも 1.5〜3 倍遅くなります。比較は最適化ありで行ってください。
- 値は CPU モデル（`--cpu`）とツールチェーンの版で変わります。最適化レベルの組み合わせごとに測り直してください。
