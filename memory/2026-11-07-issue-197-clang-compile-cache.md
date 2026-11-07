# llvm-src32 の Clang コンパイルキャッシュ

- `SRC32_CLANG_LAUNCHER` に実行ファイル名またはパスを指定すると、共通ドライバーが Clang と既存の引数をランチャーに渡す。Foundation の並列ビルドと CMake のカスタムターゲットは同じ環境変数を引き継ぐ。
- Windows の ROCm Clang 20 と ccache 4.14.2 で `driver.py -c` の IR と依存ファイルを比較した。出力パスを変えても 2 回目はヒットし、強制インクルードの内容を変えるとミスして IR が更新された。
- キャッシュを有効にして `test_toolchain.ToolchainTests` の `test_compile_only_frontend_layout`、`test_multifile_c_project`、`test_cpp_example_with_c_sdk`、`test_cmake_library_and_header_rebuild` を実行した。
- Foundation は `python foundation/boot/build.py OUT -j 6` で検証した。キャッシュの有無で `foundation.wax` と `kernel.bin` の SHA-256 は一致し、再ビルド後の `foundation.img` は `python tools/fs2/fs2.py check IMAGE` に通過した。
- ccache は Clang の翻訳単位を再利用する。Foundation 全体の所要時間には Python 側のリンク、SRC32 コード生成とディスク構築も含まれるため、ヒット率とビルド時間の両方を測る。
