# LLVM IR 生成へ MIPS コード生成用オプションを渡さない（#178）

- `foundation/run.py --disk` のビルドで `Unknown command line argument '-mips-ssection-threshold=0'` が出る原因は、共通ドライバの `-G0`。Clang が内部でこの LLVM オプションへ展開する。ROCm 6.4 同梱 Clang 20 で再現した。
- Clang は MIPS frontend の 32-bit big-endian 型・データ配置で LLVM IR のみを生成する。SRC32 の命令・静的データ配置は独自バックエンドが担当するため、MIPS small-data section のしきい値設定は不要。`--target=mips-unknown-none`、`-mabi=o32`、`-msoft-float` は維持し、`-G0` のみ除く。
- 回帰テスト `test_compile_only_frontend_layout` は実 Clang で C/C++ を O0/O1/O2 の各設定で `-c` コンパイルし、生成 IR から SRC32 WAX を作って実行する。pointer/long が 4 byte、volatile な global のバイト順と書換えが big-endian であることを検証する。freestanding C++ のエントリは既存契約どおり `extern "C"` にする。
- focused check（`tools/llvm-src32` で実行）: `python -m unittest test_toolchain.ToolchainTests.test_compile_only_frontend_layout test_toolchain.ToolchainTests.test_multifile_c_project test_toolchain.ToolchainTests.test_cpp_example_with_c_sdk`。
- Foundation 全体のビルドは `python foundation/boot/build.py target/foundation-clang178 -j 8`、生成 FS2 の整合性は `python tools/fs2/fs2.py check target/foundation-clang178/foundation.img` で確認した。元の実行用ディスクを上書きせず別出力先で検証できる。
- ディスク起動・破損検出の回帰確認: `python -m unittest discover -s foundation/tests/disk -v`。Windows では `TEMP`/`TMP` を workspace 内の既存ディレクトリに設定するとホスト既定 TEMP のアクセス制約を避けられる。
