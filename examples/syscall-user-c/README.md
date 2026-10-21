# ユーザーモード SYSCALL サンプル（C / Extension P）

拡張P（[特権レベルとシステムコール](../../doc/ja/spec_CPU_priv.md)）を使う最小のカーネルとユーザープログラムです。
1つの WAX にまとめてあるので、BIOS なしで `src32_testbench` やエミュレーターで動きます。

- `kernel.c`（スーパーバイザーモード）: 保護領域を設定し、`cpt32_run_user()` で `user.c` をユーザーモードで動かします。
  `syscall_dispatch()` が SYSCALL を処理します。
- `user.c`（ユーザーモード）: 実行できるのは `[user_main, user_code_end)` だけ、書き込めるのはユーザースタックだけです。
  グローバル変数・文字列リテラル・ライブラリ呼出しは使わず、すべて SYSCALL で頼みます。
- `syscalls.h`: このサンプル独自のシステムコール番号と、ユーザーのメモリ配置。

## 流れ

1. カーネルが PCG を初期化して `kernel: entering user mode` を表示します。
2. 領域0 = ユーザーコード（R/X）、領域1 = ユーザースタック `0x80000..0x90000`（R/W）を設定します。カーネル本体とデバイスは拒否のままです。
3. `cpt32_run_user(user_main, USER_STACK_TOP)` で `IRET` によりユーザーモードへ入ります。
4. ユーザーは次の SYSCALL を順に発行します。
   - `SYS_CALLER_MODE`: カーネルが `STATUS.PU` を見て、呼出し元がユーザーモードなら1を返します。
   - `SYS_ADD 40, 2`: 戻り値 42（R1）。
   - `SYS_PUT_HEX`: コンテキストbyte `0x0C` を文字色にして値を表示します。
   - `SYS_POKE 0x100`: カーネル領域への書き込み依頼。カーネルがユーザーのポインタを検査して拒否します。
   - `SYS_EXIT 42`: ハンドラーが `cpt32_return_to_kernel(42)` を呼び、`cpt32_run_user` が 42 を返します。
5. カーネルは S モードに戻ったことを確認し、`kernel: user exited with 0x0000002A` を表示して 42 を返します（R1=42 で HALT）。

ユーザーコードが自分の領域外へ直接書き込むと保護違反（CAUSE `0x80050002`）になります。このサンプルは例外ハンドラーを登録しないので、
既定のベクターで CPU が停止します。

## ビルドと実行

リポジトリルートから。必要なツールは [ツールチェーン README](../../tools/llvm-src32/README.md) を参照してください。

```powershell
New-Item -ItemType Directory -Force target/syscall-user | Out-Null
python tools/llvm-src32/driver.py `
  examples/syscall-user-c/user.c examples/syscall-user-c/kernel.c `
  sdk/cpt32/src/pcg.c -I sdk/cpt32/include `
  --clang 'C:/Program Files/LLVM/bin/clang.exe' -O1 `
  -o target/syscall-user/syscall_user.wax
```

CMake の場合（`$python` と `$ninja` の設定は [Hello C++](../hello-cpp/README.md) と同じ）:

```powershell
cmake -S examples/syscall-user-c -B target/syscall-user-cmake -G Ninja `
  "-DSRC32_PYTHON_EXECUTABLE=$python" `
  '-DSRC32_CLANG=C:/Program Files/LLVM/bin/clang.exe' `
  "-DCMAKE_MAKE_PROGRAM=$ninja"
cmake --build target/syscall-user-cmake --target syscall_user
```

BIOS を設定していない作業ディレクトリから実行します。

```powershell
/path/to/src32_testbench /absolute/path/syscall_user.wax 42 --cpu pipeline
```

`-O0`/`-O1`/`-O2` × Scalar/Cached/CachedRelaxed/Pipeline を
`tools/llvm-src32/test_toolchain.py` の `test_user_mode_syscall_example` で検証しています。

## 注意

- `user_code_end` は `user.c` の最後に置き、`user_main` の直後に来るようにします。カーネルは両者のアドレスで領域を決め、逆順なら 2 を返します。
- ユーザー関数から戻ってはいけません。必ず `SYS_EXIT` で終わります。
- `cpt32_run_user` は同時に1つのユーザープログラムだけを扱います（再入不可）。
- カーネルは保護されずに動くので、ユーザーから受け取ったポインタは自分で検査します（`SYS_POKE` を参照）。
- BIOS の WAX loader が受け取る **保護付き WAX**（ユーザープログラム単体）は、ドライバーの `--protected` で作ります。
  ツールチェーン README の「Extension P」を参照してください。
