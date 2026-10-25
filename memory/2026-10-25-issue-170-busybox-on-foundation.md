# BusyBox を Foundation 上で動かす（pkcl M2〜M5）

BusyBox 1.36.1 を pkcl と Newlib（どちらも外部ソース。GPL の BusyBox はリポジトリに入れない）で組み、Foundation で `busybox echo/printf/ls/cat/mkdir/cp/wc…` が動くところまで来た。契約は `doc/spec_pkcl.md` §8〜§9。

## 検証

- ビルド: `python3 tools/pkcl/busybox.py --busybox-source … --newlib-source … --fnd-wal root/lib/fnd.1.wal --out DIR`（Clang は llvmlite と同じメジャー版）。
- テスト: `SRC32_BUSYBOX_SOURCE=… SRC32_NEWLIB_SOURCE=… python3 -m unittest discover -s foundation/tests/pkcl -k busybox`（環境変数がなければ skip）。バックエンドの命令は `tools/llvm-src32` の `test_toolchain.ToolchainTests.test_funnel_shift_wide_multiply_overflow_and_unsigned_saturating_add`。

## 落とし穴

- **実行形式は 1 MiB まで、実質はシェルのヒープ（スロット 4 は 2 MiB）に収まる必要がある。** `FND_EXEC_IMAGE_MAX` を 4 MiB にしても `busybox: out of memory (-5)` になる。サイズは applet の数を決める。1.83 MB からは次で 1.04 MB にした: Newlib を `-O0` から `-Os`（posix プロファイルが `-O0` でコンパイルしていた）、hush を外す、printf／scanf の重複整形器（`_svfprintf_r`、`_vfiprintf_r`）を転送関数にする、`_scanf_float` を失敗スタブにして strtod 一式を外す。applet を足したら必ず `busybox.wax` のサイズを見る。
- **nano の printf は `%lld` に対応しない**（`ll` を解釈せず "ld" と出る）。BusyBox の `printf` が壊れるので printf は full、scanf は nano。nano はエイリアスと弱いシンボル（`_printf_float`／`_scanf_float`）を使うが、バックエンドにはどちらもない。キャッシュする IR を書き換えて対処している（`newlib.py` の `functions_for_aliases`）。
- **`ptr_to_globals` は `-DBB_GLOBAL_CONST=` が必須。** 既定では `const` で rodata に置かれ、`SET_PTR_TO_GLOBALS` の書込みが U モードのフォルト（`fault 80060002`）になる。
- **`-D__clang_major__=0`**: libbb.h の Clang 向け分岐が `asm volatile` を使うため。`barrier()` の空 asm は IR から消す（バックエンドは inline asm 非対応）。
- BusyBox の `make oldconfig` に `yes n` を渡すと、選択肢（choice）の質問で無限ループして出力がメモリを食い、プロセスが kill される。`yes ''` と `timeout` を使う。`allnoconfig` は `SH_IS_ASH=y` が既定で、NOMMU では無効のため hush に落ちる。`SH_IS_ASH=n SH_IS_HUSH=n SH_IS_NONE=y SHELL_HUSH=n` を指定する。
- 最適化したコードは `llvm.fshl/fshr`、`umul.with.overflow.i64`、`uadd.sat`、`get.rounding` などの命令を出す。足りない命令が出たら `backend.py` の `intrinsic` に足し、上のテストに入れる。i64 の `fshl/fshr` は定数のシフト量だけ対応。
- Foundation のテストで長い実行をするときは、`test_shell.STOP_TICKS` を伸ばし（既定 800 ＝ 660 フレーム）、`--frames` もそれに合わせる（約 0.83 フレーム／tick）。
- `pkcl-posix/` のラッパーヘッダは `#include_next` で Newlib のヘッダを呼ぶ。Newlib は `lstat`、`nanosleep`、`getline`、`mknod`、`SA_RESTART`、`GLOB_NOMATCH` などを隠す／持たない。足りないときはここに足す。

## まだないもの

`fork`／`vfork`（ENOSYS）、`waitpid`（ECHILD）、`pipe`（ENOSYS、G3）、`setjmp`／`longjmp`（仮実装、G7）、環境変数（G2）、pid（G1）、シグナルの配送、`glob` のディレクトリ走査。ash／hush が動くにはこれらが要る。`cp` は大きいファイルで `No space left on device`（テスト用ディスクの /tmp が小さい。pkcl の問題ではない）。
