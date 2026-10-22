# Foundation

CPT32 用の OS です。契約は [Foundation 仕様](../doc/spec_Foundation.md)、構成と開発順序は [実装案](../doc/foundation_os_plan.md) にあります。カーネルは C（[LLVM/SRC32 ツールチェーン](../tools/llvm-src32/README.md)）で書き、S モードで動きます。

## フェーズ1: 最小カーネル

[`kernel_trap` トラップスタブ](../tools/llvm-src32/README.md)（フェーズ0）の上に、次のものを実装しています。

- **トラップ**（§3.4）: IRQ、SYSCALL、U モードの例外を `kernel_trap` で振り分けます。カーネルは割り込み禁止のまま、カーネルスタック1本で動きます。U モードで fault が起きたら、そのプロセスを終了させます。終了コードは CAUSE を i32 として扱った値で、fault の EPC も記録します。idle で例外が起きた場合は `kernel_panic` になります。
- **スケジューラ**（§10.1）: 優先度は DRIVER / SERVICE / APP / IDLE の4段で、同じ優先度の中は FIFO のラウンドロビンです。tick は PeC timer ch0 の 1000 Hz（IRQ4）、タイムスライスは 10 tick です。より高い優先度のスレッドが起きたら、カーネルからユーザーへ戻る時点で切り替えます。idle は S モードのスレッドで、`HALT` を繰り返します。
- **プロセスと保護**（§5）: 7つのユーザースロットを使います。別プロセスへ切り替えるときは、§5.3 の順（全 FLAGS=0 → START/END → FLAGS）で8領域を書き換えます。終了時は全スレッドを DEAD にし、付与した RAM をゼロにします。ユーザーポインタは §5.4 の手順で検証します。
- **システムコール**（§7, §9）: `yield`、`sleep`（`FND_FOREVER` を含む）、`thread_exit`、`proc_exit`、`time_get`、`debug_write` を実装しています。ABI で定義済みでも未実装のものは `FND_E_UNSUPPORTED`、未定義の番号と context byte が 0 でない呼び出しは `FND_E_BAD_CALL` を返します。

`fnd_proc_create_builtin()` は、カーネルイメージにリンクしたユーザーコードからプロセスを作ります。WAX から作る方法は次の節を参照してください。このとき `[code_start, code_end)` が R/X、スロット上端の `ram_size` byte が R/W（スタックとデータ）になります。そのためユーザーコードでは、グローバル変数・文字列リテラル・範囲外の関数呼び出しを使えません（`examples/kernel-trap-c` と同じ制約）。システムコールは `<foundation/sys.h>` のマクロで呼ぶので、インライン展開されます。

| ファイル | 内容 |
|---|---|
| `include/foundation/abi.h`, `errors.h` | §9 の番号、§4 のエラーコード、優先度などの ABI 定数 |
| `include/foundation/sys.h` | U モード用のシステムコールマクロ |
| `include/foundation/kernel.h` | カーネルをリンクするイメージ（起動コード・デモ・テスト）向けの S モード API |
| `kernel/sched.c` | スレッド、優先度キュー、tick、sleep、idle |
| `kernel/proc.c` | プロセス、スロット、保護領域、ポインタ検証、終了処理 |
| `kernel/trap.c` | `kernel_trap` / `kernel_panic`、IRQ、システムコール |
| `kernel/wax.c`, `wax.h` | WAX ローダの本体（検証、依存解決、WAL の配置、領域の計画、束縛） |
| `demo/` | 画面に各プロセスの状態を表示するデモ |
| `tests/phase1/` | フェーズ1の受け入れテスト |
| `tests/wax/` | WAX ローダのテスト（ホスト単体テストと testbench） |

## WAX ローダ

`fnd_proc_spawn_wax(name, priority, image, size, libs, lib_count)` は、カーネルメモリ上の WAX を、その `base_addr` を含むユーザースロットに読み込んで起動します（[仕様 §9.2・§9.5](../doc/spec_Foundation.md)）。

- 形式は PROTECTED の [WAX v2](../doc/spec_WAX.md) と [WAX/WAL v3](../doc/spec_WAX_v3.md) です。v3 では、依存する WAL を `libs` から選び（同じ ID と major の中で最大の minor）、スロット内に first fit で置き、import スロットに記述子（アドレスと origin）を書きます。
- 保護領域は region 0 がスタック、その後が全画像のブロックをアドレス順に並べたものです。隣接していて権限が同じブロックだけをまとめ、8 を超えると `FND_E_REGION_LIMIT` です。
- 起動は原子的です。検証、依存解決、配置、領域とシンボルの確認がすべて通るまで、メモリにもプロセス表にも書きません。
- 入口の状態は §9.2 のとおりです（`USP = user_stack_top`、`UGP = base_addr`、他のレジスタは 0、`IE = 1`、スタックはゼロクリア）。終了時は、付与した範囲をすべてゼロにします。
- `wax.c` はカーネルの状態にもデバイスにも触れない C11 なので、ホストの C コンパイラでもビルドできます。

`proc_spawn` システムコールは、ハンドル（`PROCESS`）が実装されるフェーズ2以降に、この関数の上に作ります。そのときは §9.2 のとおり、イメージを先に staging へ複製してから検証します（現在の関数は、渡されたイメージが呼び出し中に変わらないことを前提にしています）。ライブラリ一覧をどこから得るかは未決です（§9.5）。

## ビルドと実行

カーネルは §5.1 の配置（イメージ `0x10000`、カーネルスタック頂上 `0x100000`）でビルドします。各 `user.c` では `user_code_begin` を最初の関数、`user_code_end` を最後の関数にします。

```sh
python tools/llvm-src32/driver.py foundation/demo/user.c foundation/demo/main.c \
  foundation/kernel/sched.c foundation/kernel/proc.c foundation/kernel/trap.c foundation/kernel/wax.c sdk/cpt32/src/pcg.c \
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

### WAX ローダのテスト

`tests/wax/test_wax_loader.py` は、`wax.c` をホストの C コンパイラ（`CC`）で共有ライブラリにして、[WAX v3 §8](../doc/spec_WAX_v3.md) の受理・拒否ケースを確認します。v3 の画像は `tools/wax/wax3.py` で組み立てます。

```sh
python -m unittest discover -s foundation/tests/wax -v
```

`tests/wax/main.c` は testbench 上の実行テストです。v3 を生成するコンパイラはまだないので、`tests/wax/build.py` が `*.asm` を `tools/asm` でアセンブルして v3 の WAX/WAL にし、`v2app.c` を `--protected` でビルドして、全部を `images.h` に書き出します。

- `app`/`app2`（スロット5と6）：`test.math` と `test.base` を記述子経由で呼びます。モジュール間呼び出しでの R30 の保存と復元、コールバック、64-bit 引数の順序、2 つのスロットに置いた繰り返し import、オブジェクトの import、SYSCALL 後のレジスタを確認し、成功すると 42 で終了します。WAL はプロセスごとに別の基底に置かれ、呼び出し回数のカウンタも別々です。
- `poke_write`/`poke_exec`：RX への書き込みと RW の実行が、それぞれ `0x80050002` と `0x80050004` で fault になること。
- `v2app`：C で書いた PROTECTED WAX v2。
- 失敗する起動（依存がない、minor が古い、PROTECTED でない、壊れた画像、不正な優先度）が何も変えないこと。終了後に領域がゼロになり、同じスロットへ 2 回再起動できること。
- `spin` は CPU を使い続けます。`src32_testbench` は idle スレッドの `HALT` をプログラムの終了とみなすためです。

```sh
python foundation/tests/wax/build.py target/foundation-wax
python tools/llvm-src32/driver.py foundation/tests/wax/main.c foundation/kernel/sched.c \
  foundation/kernel/proc.c foundation/kernel/trap.c foundation/kernel/wax.c \
  -I sdk/cpt32/include -I foundation/include -I target/foundation-wax \
  --origin 0x10000 --stack-top 0x100000 -O1 -o target/foundation-wax/test.wax
src32_testbench /absolute/path/target/foundation-wax/test.wax 42 --cpu pipeline --frames 90
```

`tools/llvm-src32/test_toolchain.py` の `test_foundation_wax_loader` が、`-O0`/`-O1`/`-O2` × Scalar/Cached/CachedRelaxed/Pipeline で実行します。記述子の origin をずらす、UGP を 0 にする、領域に全権限を与える、といった改変版ではこのテストが失敗することを確認しています。

## 注意と未決事項

- `src32_testbench` には UART がありません。そのため、そこで `debug_write` に 1 byte 以上を渡したり `kernel_panic` が出力したりすると、カーネル内でバスエラーになります。エミュレーター本体では UART に出力されます。
- RAM 容量の取得方法（§4.1）はまだ決めていません。スロットは既定の 16 MiB を前提にしています。
- 1プロセス1スレッドです。`thread_create`、IPC、ハンドル、`proc_spawn` システムコール、`mem_grow`、`irq_bind` などはフェーズ2以降で実装します。
- WAX の読み込み中は割り込み禁止です。`tests/wax` の起動 11 回（うち 5 回は失敗）は、-O0 の Scalar で約 16M サイクル（1 回あたり約 30 tick 分）かかります。-O2 では約 10M サイクルです。
- `tools/llvm-src32` は、byte `0x5C`（`\`）を含む `uint8_t` 配列の定数をコンパイルできません（`invalid LLVM byte escape`）。そのため `build.py` は画像を big-endian の `uint32_t` 配列として書き出します。
- timer の ACK から IRQC の W1C までの間に次の tick が来ると、IRQC の edge が失われます。カーネルは W1C の後に timer の pending をもう一度確認して、取りこぼしを防いでいます。
