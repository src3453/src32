# CI 修正: パイプライン CPU のスピン検出と phase1 の不正セレクタ

main の CI は 10/29 以降 2 系統で赤かった（fpga-foundation の取り込み後）。どちらも仕様の変更に試験が追随していなかったもの。

- `tests/spin_fast_forward.rs`（"Pipelined skipped 0"）: パイプライン CPU に静的な後方分岐予測（`predicted_taken`）が入り、正しく予測された後方分岐は redirect（flush）にならなくなった。スピン検出器は flush を「ループ先頭」の合図にしていたので、ポーリングループを一度も検出しなくなった。`spin.predicted_loops` を足して合図に含めた。L1 キャッシュ（Extension C）も同時に入っていたため、(1) `SpinState` に `cache_stall` を入れ、(2) 1 周期にミスが無いことを条件にし、(3) スキップした周回分のヒット数を `L1Cache::add_hits` で加算する。これで MMIO から見えるカウンタもスキップの有無で変わらない。
- `test_foundation_phase1_kernel`（全 CPU・全最適化で `CPU Bus Error`、R1=514）: `sys_info(6)` は FPGA 専用の `FND_SYSINFO_CPU_PERF` になった（カーネルが 0xFFFF02A0 からの 8 カウンタを読む）が、テストは「不正なセレクタ」として 6 を使っていた。エミュレータにこのレジスタはなく、カーネルが bus error になる。テストの不正セレクタを 7 に変更。エミュレータ上で `sys_info(6)` を呼ぶと今もカーネルが bus error になる点は残る（FPGA 専用の注意書きのみ）。
- 検証: `cargo test --test spin_fast_forward`、CI と同じ統合テスト一式（`strek_*` と `tb_load_live` 除外、事前に dos/tinybasic/dsfs_guest の .bin を作る）、`cd tools/llvm-src32 && python3 -m unittest test_toolchain`（52 件）。`CARGO_PROFILE_DEV_OPT_LEVEL=2`、`CLANG=clang-20`、release の `src32_testbench`。
