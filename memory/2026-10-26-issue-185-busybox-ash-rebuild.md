# ash 対応更新後の BusyBox 再ビルド

- 対象は `591a971`（pkcl の ash/fork/pipe 対応を main に統合した状態）。前回取得した BusyBox 1.36.1 と Newlib 4.4.0.20231231 を `target/busybox-work/` で再利用した。外部ソースと生成物は Git に含めない。
- ash は `--config tools/pkcl/busybox.config --config tools/pkcl/busybox_ash.config` を重ね、前回の標準構成とは別の `out-ash/` で設定から作り直す。既存 `bb/include/autoconf.h` がある出力先では設定変更が反映されないので、構成を切り替える際に古い出力先を再利用しない。
- WSL Clang 21 で `--stage compile` まで実行し、標準構成 157、ash 構成 161 翻訳単位が成功。#183 を避けるため、Newlib と pkcl および最終リンクは Windows ROCm Clang 20 で行った。作業用の `link-windows.py` は `busybox.py` の `link()` に各構成の IR を渡す。
- 標準構成の既定 `-Os` は 1,053,060 bytes になり、既存テストの 1 MiB 未満というサイズ回帰基準を超えた（#185）。ユーザーから staging 上限は現在 3 MiB と再確認されたため、実行上限と旧回帰基準を混同しないこと。最終成果物は既定の `-Os` に揃える。作業中に試した `-Oz` では 1,024,988 bytes だったが、現在の実行契約のためにそこまで縮小する必要はない。
- ash 構成は `-Os` で 1,389,012 bytes、既存の 1.5 MiB 未満基準内。`out-ash/busybox.wax` が成果物。通常起動ディスクには `/bin/busybox.wax` と同じ内容を `/bin/busybox` にも配置する。ash が applet を自己 exec するときは `CONFIG_BUSYBOX_EXEC_PATH=/bin/busybox` を使うため、拡張子付きだけでは不足する。
- ブートイメージも更新した。標準テスト用は `--stop-ticks 2000`、ash テスト用は `--stop-ticks 3000`。作業用 `verify.py` は既存 `test_busybox.py` のテストメソッドを変えず、各クラスのセットアップだけをビルド済み成果物へ差し替える。`python target/busybox-work/verify.py BusyboxAshTests` は成功（70.981 秒）。パイプ、子シェルへの環境変数、ループ、リダイレクト、サブシェル、終了コード、`||` を確認。
- 通常起動用は `python foundation/boot/build.py target/busybox-work/installed --clang 'C:/Program Files/AMD/ROCm/6.4/bin/clang.exe' --root target/busybox-work/overlay -j 8` で再生成した。停止タイマーは指定しない。今回は `overlay/bin/busybox{,.wax}` に ash 構成を置いた。`python tools/fs2/fs2.py check target/busybox-work/installed/foundation.img` は `OK`、`ls ... /bin` で両ファイルのサイズも確認した。
- 作業用 `verify.py` の配置時の上限確認は、実行 WAX と import する fnd WAL の合計が 3 MiB 以下であることに揃えた。元のテストファイルの 1 MiB／1.5 MiB 回帰 assert 自体は変更していない。既存テストメソッドの実行結果と、元のセットアップを含むテストスイート全体の結果は区別する。
- 最終の標準 `-Os` 成果物に対する `python target/busybox-work/verify.py BusyboxTests` では `test_applets` と `test_help_lists_applets` が両方成功（74.125 秒）。成果物は `out-fixed/busybox.wax`（標準）と `out-ash/busybox.wax`（ash）、通常起動用ディスクは `installed/foundation.img`（ash 入り）。
