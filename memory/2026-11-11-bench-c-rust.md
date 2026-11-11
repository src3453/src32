# C と Rust のベンチマーク（examples/bench-c-rust）

- 計測は C のハーネス `harness/harness.c` が行い、C 版・Rust 版の両方に **同じファイルを同じ `-O` で** リンクする（Rust 版は `cargo_src32.py` に `.c` と `-I` を追加入力で渡す。driver は IR 入力を再最適化しないので、ハーネスとカーネルの間で inline は起きない）。カーネルだけが言語ごとに違う。片方だけハーネスを変えると比較にならない。
- サイクルは PeC タイマ 0（free-running、prescaler 0、48 MHz）の COUNT_LO。PeC は timing-sensitive デバイスなので読み出しで遅延サイクルが反映される（`doc/spec_CPU.md` の cached-relaxed の節。pipeline と cached で実行確認済み、cached-relaxed は未確認）。testbench の `cycles=` は起動・UART 出力を含むので使わない。
- スタック深さ: llvm-src32 は全 SSA 値を spill するので `paint()` 自身のフレームが大きく、塗った領域の上端と呼出し元の SP の間に隙間ができる。カーネルを 4 KiB の volatile 配列を持つ `run_kernel` の中から呼び、カーネルのフレームが必ず塗った領域の中で始まるようにしている。基準点は配列の先頭アドレスで、`run_kernel` 自身の spill 分は空の C カーネル（`empty`）の値を引いて消す。
- コードサイズは `--save-temps` の `output.a` を `tools/asm/asm.py` の `Assembler` で組み立てて `symbols` からラベル間の長さを出す。トップレベルのラベルは `llvm_<IR 名の hex>`（C も Rust も）、`__llvm_src32_*` は関数内の局所ラベル（`__llvm_src32_start` を除く）。データは `.ORG` の後に `.BYTE` が続くラベルで、ゼロ初期化もイメージに入る。カーネルへの帰属は名前の `bench_<kernel>`（C の関数名の接頭辞、Rust v0 マングルのモジュール名 `10bench_sort` など）。Rust のモジュール名を `sort` にすると `core::slice::sort` と区別できないので `bench_` を付けている。
- Rust の opt-level は `CARGO_PROFILE_RELEASE_OPT_LEVEL` で上書きする（`cargo_src32.py` は profile に panic/lto/codegen-units/debug だけを強制する）。release なので opt-level 0 でもオーバーフロー検査はない。
- 2026-11-11 の結果（C -O2 / Rust 2、pipeline）: 添字主体の sort・matmul は Rust が 1.4〜1.5 倍遅い（境界検査）、スライス走査の crc32・sieve は Rust が 0.8 倍、nqueens・mandel は同じ。Rust のコードは panic 経路（`core::fmt` の整数 Display 約 5 KB など）で +10 KB。opt-level 0 の Rust は core が残って 190 KB。
- 検証: `python examples/bench-c-rust/run.py`（チェックサムを Python の `reference()` と比べ、違えば終了コード 1）、回帰テストは `python -m unittest discover -s tools/llvm-src32 -p test_rust.py -k bench`（Rust がなければ skip）。
