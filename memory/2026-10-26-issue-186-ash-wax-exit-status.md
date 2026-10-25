# ash の外部 WAX エラー終了を SIGSEGV と誤認しない

- ユーザー報告の `cat.wax` で再現。ash から `/bin/cat.wax` を引数なしで実行すると、`usage: cat path...` の後に `Segmentation fault`、`$?=139` となった。存在しないファイルでも同じ。存在するファイルだけを読むと成功する。
- Foundation の native コマンドは通常のエラーコードを `fnd_exit` に渡す。`cat` の引数不足は `FND_E_BAD_ARG=-2`、ファイルなしは `FND_E_NOT_FOUND=-10`。`pkcl/posix/process.c` の旧 `wait_status` は 0〜255 以外を SIGSEGV にしたため、通常のエラー終了をフォールトと誤認した。実際のメモリ破壊ではない。
- 変換契約を整理：`FND_E_KILLED=-20` は SIGKILL、Foundation の予約範囲である -256 以下の CAUSE は SIGSEGV、それ以外は下位 8 bit の通常終了にする。-2 は254、-10 は246、256 は0。Foundation 側のエラー値と native コマンドの動作は変えない。
- `guest_posix.c` で -1/-2/-10/-64/-255/0/255/256/259 の終了状態、実際の未マップ領域への書き込みフォールト、kill による SIGKILL を確認する。表示だけを抑えて本当のフォールトを隠す修正にしない。
- `test_busybox.py` の ash テストに、native `cat.wax` の成功、引数不足、正常なファイルを表示した後に次のファイルで失敗するケースを追加した。終了値と `Segmentation fault` が出ないことの両方を確認する。ash の既存パイプラインテストも維持する。
- 検証：`SRC32_NEWLIB_SOURCE` と `CLANG` を設定して `python -m unittest discover -s foundation/tests/pkcl -k test_posix_process_and_pipe -v` は成功（40.953 秒）。Windows の TEMP/TMP は `target/busybox-work/temps` に設定した。
- BusyBox の IR は変更不要で、更新された pkcl を再リンクする。作業用 `python target/busybox-work/link-windows.py out-ash`（Newlib 4.4、ROCm Clang 20）で ash 版を再生成し、`overlay/bin/busybox{,.wax}` を差し替えて通常起動ディスクも再生成する。外部ソースと生成物は Git 管理しない。
- `python target/busybox-work/verify.py BusyboxAshTests` で、ビルド済み成果物を使うセットアップから外部 WAX 終了状態・既存パイプラインの 2 テストメソッドが成功（75.181 秒）。修正版 ash WAX は 1,389,084 bytes。`installed/foundation.img` は再生成後の FS2 check も `OK`。標準構成の `out-fixed/busybox.wax` も同じ pkcl 修正で再リンクした。
