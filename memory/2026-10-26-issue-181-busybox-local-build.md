# BusyBox 1.36.1 の取得と Windows/WSL でのビルド

- 外部ソースは公式 `https://busybox.net/downloads/busybox-1.36.1.tar.bz2` を取得し、同配布元の `.sha256` と照合した。SHA-256 は `b8cc24c9574d809e7279c3be349795c5d5ceb6fdf19ca709f80cde50e47de314`。ソース・アーカイブ・生成物は無視対象の `target/busybox-work/` に置き、Git に含めない。
- BusyBox のホストビルドは Unix の GCC/make/bison/ar/sh を必要とする。今回は Ubuntu WSL、Clang 21、作業用 venv の llvmlite 0.50.0 で BusyBox の 157 翻訳単位をコンパイルした。Windows 側の Python/ROCm Clang 20 では Foundation のブートイメージを構築した。
- Newlib 4.4.0.20231231 と 4.5.0.20241231 はどちらも Clang 21 で `_strtod_l` に `llvm.is.fpclass.f64` を出し、SRC32 バックエンドが未対応のためリンクが失敗する（#183）。Newlib の版だけを戻しても回避できない。Windows 側の ROCm Clang 20 で Newlib 4.4 と最終リンクを行うことで回避できた。
- GNU make 4.4 の `MAKEFLAGS` には jobserver オプションが入り、BusyBox 1.36.1 の `findstring s,$(MAKEFLAGS)` が silent と誤判定する。`V=1` だけではコンパイルコマンドがログに出ず、変換対象が 0 件になり `@main` 未定義でリンクに失敗する。`tools/pkcl/busybox.py` のホストビルドに `quiet=` を追加して回避した（#181）。BusyBox 本体へのパッチは不要。
- WSL の `TMPDIR` は Linux 側の `/tmp` を使う。`/mnt/c/` 配下では make の jobserver FIFO 作成が `Operation not supported` になる。
- `--newlib-source` は絶対パスで渡す。各翻訳単位のコンパイルは出力先の `bb/` を cwd にして起動されるため、リポジトリからの相対パスはそこでは解決できない。
- 再実行コマンド（Ubuntu WSL、リポジトリの絶対パスを `REPO` に設定）：

  ```sh
  target/busybox-work/venv/bin/python tools/pkcl/busybox.py \
    --busybox-source "$REPO/target/busybox-work/busybox-1.36.1" \
    --newlib-source "$REPO/target/busybox-work/newlib-4.4.0.20231231" \
    --fnd-wal "$REPO/target/busybox-work/boot/root/lib/fnd.1.wal" \
    --out target/busybox-work/out-fixed --clang /usr/bin/clang-21 -j 8
  ```

  上記の Clang 21 経路は現在 #183 の未対応命令でリンクに失敗する。生成済み `out-fixed/ir/*.ll` は Windows 側の `tools/pkcl/busybox.py` の `link()` に渡して再利用できる（作業用 `target/busybox-work/link-windows.py`）。

- 既存ゲストテストは `foundation/tests/pkcl/test_busybox.py`。通常は `SRC32_BUSYBOX_SOURCE` と `SRC32_NEWLIB_SOURCE` を設定して `python -m unittest discover -s foundation/tests/pkcl -k busybox -v` を実行する。今回は永続成果物を残すため、作業用 `target/busybox-work/verify.py` から既存の 2 テストをそのまま呼び、ビルド済み WAX とブートイメージを再利用する。
- 生成した `target/busybox-work/out-fixed/busybox.wax` は 1,043,456 bytes で、既存の 1 MiB 未満のサイズ回帰基準を満たした。
- `python target/busybox-work/verify.py` で既存の `test_applets` と `test_help_lists_applets` が両方成功（73.845 秒）。`echo`、整数／浮動小数の `printf`、`basename`、`dirname`、存在しないファイルの `cat` エラー、help の applet 一覧を確認した。ビルド・ゲスト実行ログは同作業ディレクトリに残してある。
- 通常起動用は `python foundation/boot/build.py target/busybox-work/installed --clang 'C:/Program Files/AMD/ROCm/6.4/bin/clang.exe' --root target/busybox-work/overlay -j 8` で構築し、`overlay/bin/busybox.wax` を取り込んだ。テスト用と違い `--stop-ticks` を指定しない。`python tools/fs2/fs2.py check target/busybox-work/installed/foundation.img` は `OK`。
