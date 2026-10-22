# Foundation

CPT32 用の OS です。契約は [Foundation 仕様](../doc/spec_Foundation.md)、構成と開発順序は [実装案](../doc/foundation_os_plan.md) にあります。カーネルは C（[LLVM/SRC32 ツールチェーン](../tools/llvm-src32/README.md)）で書き、S モードで動きます。

## フェーズ1: 最小カーネル

[`kernel_trap` トラップスタブ](../tools/llvm-src32/README.md)（フェーズ0）の上に、次のものを実装しています。

- **トラップ**（§3.4）: IRQ、SYSCALL、U モードの例外を `kernel_trap` で振り分けます。カーネルは割り込み禁止のまま、カーネルスタック1本で動きます。U モードで fault が起きたら、そのプロセスを終了させます。終了コードは CAUSE を i32 として扱った値で、fault の EPC も記録します。idle で例外が起きた場合は `kernel_panic` になります。
- **スケジューラ**（§10.1）: 優先度は DRIVER / SERVICE / APP / IDLE の4段で、同じ優先度の中は FIFO のラウンドロビンです。tick は PeC timer ch0 の 1000 Hz（IRQ4）、タイムスライスは 10 tick です。より高い優先度のスレッドが起きたら、カーネルからユーザーへ戻る時点で切り替えます。idle は S モードのスレッドで、`HALT` を繰り返します。
- **プロセスと保護**（§5）: 7つのユーザースロットを使います。別プロセスへ切り替えるときは、§5.3 の順（全 FLAGS=0 → START/END → FLAGS）で8領域を書き換えます。終了時は全スレッドを DEAD にし、付与した RAM をゼロにします。ユーザーポインタは §5.4 の手順で検証します。
- **システムコール**（§7, §9）: `yield`、`sleep`（`FND_FOREVER` を含む）、`thread_exit`、`proc_exit`、`time_get`、`debug_write` を実装しています。ABI で定義済みでも未実装のものは `FND_E_UNSUPPORTED`、未定義の番号と context byte が 0 でない呼び出しは `FND_E_BAD_CALL` を返します。

WAX ローダ（`proc_spawn`）はフェーズ3なので、まだありません。フェーズ1では `fnd_proc_create_builtin()` を使い、カーネルイメージにリンクしたユーザーコードからプロセスを作ります。このとき `[code_start, code_end)` が R/X、スロット上端の `ram_size` byte が R/W（スタックとデータ）になります。そのためユーザーコードでは、グローバル変数・文字列リテラル・範囲外の関数呼び出しを使えません（`examples/kernel-trap-c` と同じ制約）。システムコールは `<foundation/sys.h>` のマクロで呼ぶので、インライン展開されます。

| ファイル | 内容 |
|---|---|
| `include/foundation/abi.h`, `errors.h` | §9 の番号、§4 のエラーコード、優先度などの ABI 定数 |
| `include/foundation/sys.h` | U モード用のシステムコールマクロ |
| `include/foundation/kernel.h` | カーネルをリンクするイメージ（起動コード・デモ・テスト）向けの S モード API |
| `kernel/sched.c` | スレッド、優先度キュー、tick、sleep、idle |
| `kernel/proc.c` | プロセス、スロット、保護領域、ポインタ検証、終了処理 |
| `kernel/trap.c` | `kernel_trap` / `kernel_panic`、IRQ、システムコール |
| `demo/` | 画面に各プロセスの状態を表示するデモ |
| `tests/phase1/` | フェーズ1の受け入れテスト |

## ビルドと実行

カーネルは §5.1 の配置（イメージ `0x10000`、カーネルスタック頂上 `0x100000`）でビルドします。各 `user.c` では `user_code_begin` を最初の関数、`user_code_end` を最後の関数にします。

```sh
python tools/llvm-src32/driver.py foundation/demo/user.c foundation/demo/main.c \
  foundation/kernel/sched.c foundation/kernel/proc.c foundation/kernel/trap.c sdk/cpt32/src/pcg.c \
  -I sdk/cpt32/include -I foundation/include --origin 0x10000 --stack-top 0x100000 -O1 \
  -o target/foundation_demo.wax
cargo run --release --bin cpt32 -- target/foundation_demo.wax
```

デモでは、yield しない3つのカウンタが 10 tick ずつ交互に進みます。`crash` は2〜3秒後にカーネル領域へ書き込み、`exit 80050002`（保護違反）で終了します。残りのプロセスはそのまま動き続けます。SERVICE 優先度の `ticker` は 250 tick ごとに起き、そのたびにカウンタを割り込んで実行されます。

CMake の場合（`$python` と `$ninja` は [Hello C++](../examples/hello-cpp/README.md) と同じ）:

```sh
cmake -S foundation -B target/foundation -G Ninja -DSRC32_PYTHON_EXECUTABLE=$python -DSRC32_CLANG=/path/to/clang
cmake --build target/foundation
```

これで `foundation_demo.wax` と `foundation_phase1_test.wax` ができます。

## 受け入れテスト

`tests/phase1` は、[実装案 §8](../doc/foundation_os_plan.md) のフェーズ1の条件を確認します。tick フックの中で検査し、成功すると R1=42、失敗すると `0x100 + 失敗数` で停止します。

- yield しない U モードのカウンタ A・B が両方とも進むこと。
- 別プロセスが A の領域に書き込もうとすると、保護違反（`0x80050002`）でそのプロセスだけが終了すること。A への書き込みは届かず、終了したプロセスの RAM はゼロになり、A と B はその後も進むこと。
- SERVICE プロセスについて、次のことを確認します。
  - `sleep(7)` が 7〜8 tick で戻ること（APP を割り込んで実行される）。
  - 不正なポインタは `FND_E_ACCESS` になること。
  - 未定義の番号や context 付きの呼び出しは `FND_E_BAD_CALL`、未実装の呼び出しは `FND_E_UNSUPPORTED` になること。
  - `time_get` で tick 数を取得できること。
  - `proc_exit` の終了コードが記録されること。
- SERVICE プロセスの終了後は、10 tick のスライスだけで A と B が交互に進むこと。

```sh
src32_testbench /absolute/path/foundation_phase1_test.wax 42 --cpu pipeline --frames 30
```

`tools/llvm-src32/test_toolchain.py` の `test_foundation_phase1_kernel` が、`-O0`/`-O1`/`-O2` × Scalar/Cached/CachedRelaxed/Pipeline で実行します。スライスによる切り替えや保護領域の切り替えを無効にした改変版では、このテストが失敗することも確認しています。

## 注意と未決事項

- `src32_testbench` には UART がありません。そのため、そこで `debug_write` に 1 byte 以上を渡したり `kernel_panic` が出力したりすると、カーネル内でバスエラーになります。エミュレーター本体では UART に出力されます。
- RAM 容量の取得方法（§4.1）はまだ決めていません。スロットは既定の 16 MiB を前提にしています。
- 1プロセス1スレッドです。`thread_create`、IPC、ハンドル、`proc_spawn`、`irq_bind` などはフェーズ2以降で実装します。
- timer の ACK から IRQC の W1C までの間に次の tick が来ると、IRQC の edge が失われます。カーネルは W1C の後に timer の pending をもう一度確認して、取りこぼしを防いでいます。
