# 文脈切替トラップスタブのサンプル（C / `kernel_trap`）

Foundation OS のフェーズ 0 の確認用です。`tools/llvm-src32` が生成する文脈切替トラップスタブ（[Foundation 仕様 §3](../../doc/spec_Foundation.md)）を使い、最小のカーネルが複数の文脈を切り替えます。
カーネルとユーザーコードを 1 つの WAX にまとめてあるので、BIOS なしで `src32_testbench` で動きます。

- `kernel.c`（S モード）: `kernel_trap()` が全 IRQ と例外を受けます。保存済みの `cpt32_ctx_frame` を検査して、次に走らせる frame を返します。`kernel_panic()` は、カーネル実行中のトラップで呼ばれます。
- `user.c`（U モード）: 実行できるのは `[user_code_begin, user_code_end)` だけ、書き込めるのは 4 本のユーザースタック（`0x80000..0xA0000`）だけです。
- `trap_demo.h`: このサンプル独自のシステムコール番号（Foundation の ABI ではありません）とスタック配置。

## 流れ

1. `main` が保護領域を設定し、タスク A と B の frame を作って `cpt32_ctx_resume()` で A を開始します。`main` には戻りません。
2. **協調切替**: A と B は `task_mix` を実行します。3 つの値を保持したまま `SYS_YIELD` を発行するたびに、もう一方へ切り替わります。カーネルは同じ計算を自分でも行い、引数がすべて一致することを確認します。レジスタやスタックが壊れていれば、ここで不一致になります。
3. **S モード idle**: 両方が `SYS_EXIT` すると、カーネルは S モードの idle 文脈（`IE=1` で `HALT` を繰り返す）へ戻り、PeC timer ch0 を periodic（20,000 cycles、IRQ4）で開始します。最初のタイマー IRQ では、S からのトラップとして `r[28]` が idle スタックを指していることを確認します。
4. **プリエンプション**: タスク C と D は `task_spin` を実行します。自分からは切り替えず、1024 回ごとに `SYS_REPORT` するだけです。カーネルはタイマー IRQ のたびに C と D を入れ替えます。両方が 3 回以上報告し、横取りが 4 回以上起きたら終了します。
5. カーネルは S モード・`IE=0` の frame を返して、R1 = 42（検査の失敗があれば `0x100 + 件数`）で `HALT` します。

## ビルドと実行

リポジトリルートから実行します。必要なツールは [ツールチェーン README](../../tools/llvm-src32/README.md) を参照してください。

```powershell
python tools/llvm-src32/driver.py `
  examples/kernel-trap-c/user.c examples/kernel-trap-c/kernel.c `
  -I sdk/cpt32/include --clang 'C:/Program Files/LLVM/bin/clang.exe' -O1 `
  -o target/kernel-trap/kernel_trap.wax
```

CMake の場合（`$python` と `$ninja` の設定は [Hello C++](../hello-cpp/README.md) と同じ）:

```powershell
cmake -S examples/kernel-trap-c -B target/kernel-trap-cmake -G Ninja `
  "-DSRC32_PYTHON_EXECUTABLE=$python" `
  '-DSRC32_CLANG=C:/Program Files/LLVM/bin/clang.exe' `
  "-DCMAKE_MAKE_PROGRAM=$ninja"
cmake --build target/kernel-trap-cmake --target kernel_trap
```

BIOS を設定していない作業ディレクトリから実行します。

```powershell
/path/to/src32_testbench /absolute/path/kernel_trap.wax 42 --cpu pipeline
```

`-O0`/`-O1`/`-O2` × Scalar/Cached/CachedRelaxed/Pipeline を、`tools/llvm-src32/test_toolchain.py` の `test_kernel_trap_context_switch_example` で検証しています。

## 注意

- 全タスクが 1 組の保護領域を共有しているので、タスク同士は互いのスタックに書き込めます。プロセスごとの領域切り替えは、フェーズ 1 のカーネルで扱います。
- `halt_code` はカーネルの読み取り専用データに置いた `HALT` 命令です。S モードの frame を `IE=0` でそこへ戻すと、R1 を保ったまま停止できます。
