# Foundation

CPT32 用の OS です。契約は [Foundation 仕様](../doc/spec_Foundation.md)、構成と開発順序は [実装案](../doc/foundation_os_plan.md) にあります。カーネルは C（[LLVM/SRC32 ツールチェーン](../tools/llvm-src32/README.md)）で書き、S モードで動きます。

## フェーズ1: 最小カーネル

[`kernel_trap` トラップスタブ](../tools/llvm-src32/README.md)（フェーズ0）の上に、次のものを実装しています。

- **トラップ**（§3.4）: IRQ、SYSCALL、U モードの例外を `kernel_trap` で振り分けます。カーネルは割り込み禁止のまま、カーネルスタック1本で動きます。U モードで fault が起きたら、そのプロセスを終了させます。終了コードは CAUSE を i32 として扱った値で、fault の EPC も記録します。idle で例外が起きた場合は `kernel_panic` になります。
- **スケジューラ**（§10.1）: 優先度は DRIVER / SERVICE / APP / IDLE の4段で、同じ優先度の中は FIFO のラウンドロビンです。tick は PeC timer ch0 の 1000 Hz（IRQ4）、タイムスライスは 10 tick です。より高い優先度のスレッドが起きたら、カーネルからユーザーへ戻る時点で切り替えます。idle は S モードのスレッドで、`HALT` を繰り返します。
- **プロセスと保護**（§5）: 7つのユーザースロットを使います。別プロセスへ切り替えるときは、§5.3 の順（全 FLAGS=0 → START/END → FLAGS）で8領域を書き換えます。終了時は全スレッドを DEAD にし、付与した RAM をゼロにします。ユーザーポインタは §5.4 の手順で検証します。
- **システムコール**（§7, §9）: `sys_info`（sel 0 と 1）、`yield`、`sleep`（`FND_FOREVER` を含む）、`thread_exit`、`proc_exit`、`time_get`、`debug_write` を実装しています。ABI で定義済みでも未実装のものは `FND_E_UNSUPPORTED`、未定義の番号と context byte が 0 でない呼び出しは `FND_E_BAD_CALL` を返します。フェーズ2の部分は次の節です。

## フェーズ2: ハンドル、IPC、プロセスの起動

- **ハンドル**（§6）: PORT、CHANNEL、PROCESS をプロセスごとの表（1〜63）で持ちます。`handle_close`、`handle_dup`、`XFER_HANDLE` による移動、終了時の一括クローズを実装しています。THREAD はまだありません（`thread_create` が未実装）。
- **IPC**（§8）: `port_create`、`chan_create`、`ipc_call`、`ipc_recv`、`ipc_reply`、`ipc_reply_recv`、`svc_register`、`svc_lookup`（badge の bit31 付き）、タイムアウト（§7.1）、最後のハンドルが閉じた CHANNEL の `CLIENT_GONE` 通知、相手がいなくなったときの `FND_E_PEER_GONE`。実装は `kernel/ipc.c` です。
- **プロセス**（§9）: `proc_spawn`（イメージと WAL の一覧 `libs` を staging に複製してから検証、ハンドルを子の 1 番から dup、PROCESS ハンドルを返す）、起動ブロック（§9.3）と `proc_startup`、`wait`、`proc_kill`、`mem_grow`（ヒープは最後の RW ブロックの末尾を 8 byte 境界に切り上げた位置から）、`sys_info` の `parent`。親が PROCESS ハンドルを閉じた終了済みのプロセスは、プロセス表から消えます。
- **同梱モジュール**（§11）: FNDBOOT がまだないので、`boot/main.c` がカーネルにリンクした console、fs、init を `fnd_proc_spawn_module` で起動します（argv = [名前]、cwd = `/`、役割なし）。
- まだないもの：`thread_create`、`irq_bind`、`irq_ack`、`blk_io`（`FND_E_UNSUPPORTED`）。

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
| `kernel/ipc.c`, `include/foundation/ipc.h` | ハンドル、IPC、サービス名、`wait`、`proc_kill`（§6〜§9） |
| `kernel/fndboot.c` | FNDBOOT のモジュール表の検査と、同梱モジュールの起動（§11） |
| `include/foundation/api.h`, `fnd.h`, `lib/fnd/` | libfnd（[API 仕様 §11](../doc/spec_Foundation_API.md)）のうち、今あるもの。WAL「fnd」1.0（`/lib/fnd.1.wal`）としてビルドし、export は `lib/fnd/fnd.exports` |
| `services/console/` | console サービス（API 仕様 §8、PCG と PeC キーボード） |
| `services/bootfs/` | 暫定の読み取り専用 fs サービス（`/bin` と `/lib`、API 仕様 §5・§6） |
| `services/init/` | init（API 仕様 §12）。`/bin/shell.wax` を起動し、終了したら起動し直す |
| `shell/` | シェル（[シェル仕様](../doc/spec_Foundation_shell.md)）。ディスパッチャで、組み込みは `cd`、`set`、`exit`、`help` |
| `bin/` | 標準コマンド（シェル仕様 §10.3）のうち fs のいらないもの：`clear`、`echo`、`free`、`ps`、`pwd`、`top`、`uname` |
| `boot/` | カーネルの入口（`main.c`）、stage1（`stage1.a`）、FNDBOOT の packer（`fndboot.py`）、起動イメージとディスクイメージのビルドスクリプト（`build.py`） |
| `demo/` | 画面に各プロセスの状態を表示するデモ |
| `tests/phase1/` | フェーズ1の受け入れテスト |
| `tests/wax/` | WAX ローダのテスト（ホスト単体テストと testbench） |
| `tests/wal/` | C から作った WAL と WAX v3 を、カーネルで実行するテスト |
| `tests/shell/` | シェルのテスト（testbench にキー入力を送り、PCG の画面を比べる） |
| `tests/disk/` | ディスク起動のテスト（BIOS → stage1 → FNDBOOT → シェル、stage1 とカーネルの拒否ケース） |

## WAX ローダ

`fnd_proc_spawn_wax(name, priority, image, size, libs, lib_count, grants, grant_count)` は、カーネルメモリ上の WAX を、その `base_addr` を含むユーザースロットに読み込んで起動します（[仕様 §9.2・§9.5](../doc/spec_Foundation.md)）。

- 形式は PROTECTED の [WAX v2](../doc/spec_WAX.md) と [WAX/WAL v3](../doc/spec_WAX_v3.md) です。v3 では、依存する WAL を `libs` から選び（同じ ID と major の中で最大の minor）、スロット内に first fit で置き、import スロットに記述子（アドレスと origin）を書きます。
- 保護領域は region 0 がスタック、その後が全画像のブロックをアドレス順に並べたものです。隣接していて権限が同じブロックだけをまとめ、8 を超えると `FND_E_REGION_LIMIT` です。
- `grants` は MMIO の付与（§9.2 の `grants`、§10.3）で、画像の後の region になります（§5.3 の手順 4）。§10.3 の窓に収まらないものは `FND_E_ACCESS`、R（1）と RW（3）以外の perm は `FND_E_BAD_ARG` です。VRAM と PCMRAM は 1 プロセスにしか付与できず、動いている別のプロセスが持っていれば `FND_E_BUSY` です（§10.3 は誤りのコードを定めていないので、カーネルのコードから選んだ）。付与した範囲は §5.4 のポインタ検証の対象外で、終了時にゼロにもしません。
- 起動は原子的です。検証、依存解決、配置、領域とシンボルの確認がすべて通るまで、メモリにもプロセス表にも書きません。
- 入口の状態は §9.2 のとおりです（`USP = user_stack_top`、`UGP = base_addr`、他のレジスタは 0、`IE = 1`、スタックはゼロクリア）。終了時は、付与した範囲をすべてゼロにします。
- `wax.c` はカーネルの状態にもデバイスにも触れない C11 なので、ホストの C コンパイラでもビルドできます。

`proc_spawn` システムコールはこの関数の上にあり、§9.2 のとおりイメージと `libs` の WAL を先に staging（`0x00100000` 以降のカーネルヒープ）へ複製してから検証します。WAL は `/lib/<image_id>.<abi_major>.wal` に置き、起動する側が読んで `libs` で渡します（§9.5）。libfnd では `fnd_proc_exec` が依存表をたどって読みます。コピーはワード単位で、1 バイトずつだとシェルの読み込みだけで約 15 frame かかっていました。

## シェルと起動の構成

起動すると、カーネルが console（スロット 1）、fs（スロット 3）、init（スロット 0）を起動し、init が `/bin/shell.wax`（スロット 4）を起動します。シェルは [シェル仕様](../doc/spec_Foundation_shell.md) 0.2 のディスパッチャで、`cd`、`set`、`exit`、`help` 以外のコマンドは `PATH`（既定 `/bin`）から `<name>.wax` を探し、`fnd_proc_exec` でスロット 5 に起動して終了を待ちます。

- **libfnd の共有**：シェルと `/bin` のコマンドは、libfnd を WAL「fnd」1.0（`/lib/fnd.1.wal`）として import する WAX v3 です。コマンドは 1 つ約 9 KiB です（静的リンクでは約 38 KiB でした）。init と各サービスは、fs より先にカーネルが起動するので、libfnd を静的にリンクした PROTECTED WAX v2 です。
- **できること**：字句と構文（§3、§4）、変数（§5.1）、外部コマンドの解決と起動（§7。パス指定、`PATH`、大文字小文字を区別しない名前）、リダイレクトの解析と `2>&1`、行編集（§9.2）、エラーメッセージ（§11.1）、fault で終わったプロセスの報告。プロンプトは cwd と `> ` です。
- **まだできないこと**：暫定 fs は読み取り専用なので、ファイルへの書き込みのリダイレクトは `read-only`（`FND_E_READ_ONLY`）、ワイルドカードとスクリプト（`.fsh`）は `FND_E_UNSUPPORTED` です。`ls`、`cat`、`cp`、`rm`、`mv`、`mkdir`、`rmdir` は fs サービスの完成後に `/bin` へ加えます。`thread_create` がないので、Ctrl+C は行編集中の入力を捨てるだけです。
- **暫定の部分**：`services/bootfs` は、ビルド時に埋め込んだ `/bin` と `/lib` だけを持つ読み取り専用の fs です（`boot/build.py` が `bootfs_files.h` を作る）。ディスク上の FS2 を読む fs サービスができたら置き換えます。同じファイルは `OUT/root` にも書き出すので、ディスクイメージの元にできます。
- **起動イメージ**：`boot/main.c` は FNDBOOT（§11）のモジュール表（console、fs、init の順）を spawn します。console の MMIO の付与と各モジュールの `caps` はモジュール表に書いてあります。ディスク起動では stage1 が読み込んだ FNDBOOT を使い、`foundation.wax` では `boot/build.py` がカーネルを含まない FNDBOOT（`fndboot_image.h`）をリンクします（次の節）。シェルの再起動は init が行うので、モジュールを起動した後は staging 領域を `proc_spawn` が使います。
- **console**：`services/console` が PCG（SCREEN_MODE 1、80×30、8×8）と PeC のキーボードを MMIO で扱います（API 仕様 §8）。`irq_bind` がないので、キーは IPC の合間と 4 tick ごとに FIFO から読み、256 キーまで先行入力をためます。`CON_READ_KEY` の `NOWAIT`（`fnd_con_pollkey`、`top` が使う）は待たずに答えます。`CON_SET_COLOR` は `FND_E_UNSUPPORTED` です。
- `uname`、`free`、`ps`、`top` と起動時の表示は `sys_info` を使います。`ram_bytes` は §5.1 の前提の 16 MiB、`free_bytes` は動いているプロセスが占めていないユーザースロットの合計、`boot_drive` はディスク起動では stage1 から渡された drive、`foundation.wax` では 0 です。

```sh
python foundation/run.py
```

`/bin/top` は CPU 使用率（user / kernel 概算 / idle）、メモリ容量と CPU 使用率順のプロセス一覧を 1 秒ごとに更新します。`q` または Ctrl+C で戻ります。`top -n 1` は 1 回分を表示して終了します。kernel の値は C ハンドラの計測で、context save/restore を含む厳密な OS オーバーヘッドではありません。

リポジトリルートで実行してください。スクリプトを絶対パスで指定すれば、別の作業ディレクトリからも使えます。Python と llvmlite、Clang、Rust/Cargo が必要です（[ツールチェーンの準備](../tools/llvm-src32/README.md#必要なツール)）。`--clang PATH` で Clang を指定でき、`--build-only` はビルドと設定生成だけを行います。

`--arg` 以降の引数は、そのままエミュレーターへ渡します。`run.py` のオプションは `--arg` より前に指定してください。空白を含む値は通常のシェル引数と同様に引用符で囲みます。

```sh
python foundation/run.py --disk --arg --debug-gui --start-paused --cpu pipeline
```

このコマンドはシェルとカーネルを `target/foundation-boot/foundation.wax` にビルドし、release エミュレーターをビルドして起動します。作業用一時ファイルも同ディレクトリ内に置きます。`target/foundation-boot/run/config.yaml` に 16 MiB RAM と付属の `assets/UNSCII.F08` フォントを設定し、そのディレクトリをエミュレーターの作業ディレクトリにします。普段使うルートの `config.yaml` は変更しません。

手動で起動する場合は `python foundation/boot/build.py target/foundation-boot` で画像を作り、BIOS 未設定の作業ディレクトリから画像の絶対パスをエミュレーターへ渡してください。`bios.image` があると WAX の入口より BIOS が優先されます。PCG の文字を表示するには、作業ディレクトリの `config.yaml` に `pcg.font`（例：`assets/UNSCII.F08` の絶対パス）が必要です。

`tests/shell/test_shell.py` は、`--stop-ticks 3000` でビルドした起動イメージを `src32_testbench` で 330 frame 動かします。`--type FRAME=TEXT` でその frame にキーを送り（`\n` Enter、`\b` Backspace、`\L` `\R` `\U` `\D` 矢印、`\H` Home、`\E` End、`\X` Delete、`\C` Ctrl+C、`\\` バックスラッシュ）、`--dump-pcg` で最後の画面（SCROLL_Y を反映した 30 行）を出力させて比べます。GUI は Xvfb の下では黒い画面になるので、画面の確認はこの方法で行います。

```sh
python -m unittest discover -s foundation/tests/shell -v
```

## ディスク起動

`python foundation/boot/build.py OUT` は `foundation.wax` のほかに、BIOS から起動できる FS2 ディスク `OUT/foundation.img` も作ります。起動の流れは [仕様 §11](../doc/spec_Foundation.md)、[FS2 §3](../doc/spec_FS2.md)、[BIOS §3](../doc/spec_BIOS.md) のとおりです。

1. BIOS が IDC dd0〜dd3 のうち署名のある最初の drive の LBA 0（`boot/stage1.a`、512 bytes）を `0x0` に読み、R1 = drive で分岐します。
2. stage1 は superblock（LBA 1）を検査し（magic、version、flags = 0 なので DIRTY = 0、`total_blocks` ≤ 容量、`boot_start` = 2、`bitmap_start` = 2 + `boot_blocks` ≤ `data_start` ≤ `total_blocks`、0 < `boot_size` ≤ 2 MiB、予約領域に収まること）、予約ブート領域から FNDBOOT を `0x00200000` に読みます。FNDBOOT のヘッダを検査してカーネルを `0x00010000` へコピーし、R1 = drive、R2 = `0x00200000` で `kernel_entry` へ分岐します。失敗すると R1 = 1（IDENTIFY か superblock の読み込み）、2（superblock）、3（FNDBOOT）で HALT します。画面には何も出ません（510 bytes に収めるため）。
3. カーネル（`main(drive, fndboot)`）は `boot_drive` を記録し（`sys_info`）、`fnd_boot_check` でモジュール表全体を検査してから、表の順に `fnd_boot_spawn` します。表が不正なら main が `FND_E_FORMAT` を返し、カーネルはその値を R1 に入れて HALT します。

ディスクの中身は次のとおりです。既定は 8 MiB（16384 blocks、inode 256）で、予約ブート領域は stage1 が受け付ける最大の 2 MiB（4096 blocks）です。FNDBOOT が大きくなっても再 format は要りません。

| 場所 | 中身 |
|---|---|
| block 0 | `stage1.bin`（署名 `55 AA` 付き） |
| 予約ブート領域 | `fndboot.bin`：ヘッダ、モジュール表（console、fs、init。優先度 SERVICE、console には MMIO 付与）、`kernel.bin`（`0x10000` の raw image）、各モジュールのイメージ |
| `/bin/*.wax`、`/lib/fnd.1.wal` | シェル、標準コマンド、libfnd（mode `0x003D`）。`OUT/root` と同じもの。fs サービスがディスクを読めるようになるまでは、暫定 fs が同じファイルをイメージから返します |
| `--root DIR` | ホストの `DIR` の中身をそのままルートへ置きます（例：`DIR/bin/*.wax`、`DIR/etc/startup`）。`/bin` の下は mode `0x003D` です |

```sh
python foundation/run.py --disk
```

で、BIOS（`assets/bios.rom`）と dd0 の HDD（読み取り専用）を設定してエミュレーターを起動します。プログラムには先頭 word が 0 の 4 bytes を渡すので、BIOS は RAM 起動を飛ばしてディスクから起動します。POST の表示を 120 frame 保持した後、HDD なら一瞬で、FDD（`type: "fdd"`）なら 6 秒ほどで（62,500 B/s）シェルが出ます。ディスクの中身は `tools/fs2/fs2.py`（[README](../tools/fs2/README.md)）で見たり変えたりできます。

```sh
python tools/fs2/fs2.py ls target/foundation-boot/foundation.img /bin
python tools/fs2/fs2.py check target/foundation-boot/foundation.img
```

`tests/disk/test_disk_boot.py` は `--stop-ticks 4000` でビルドしたディスクを、BIOS と `src32_testbench` で起動します。dd0 を空にして dd1 から起動し（stage1 が R1 の drive を使うことの確認）、`uname` を打って画面を比べます。superblock の DIRTY と配置、FNDBOOT の magic と size、モジュール表の予約欄を壊したディスクでは、それぞれ決まった R1 で止まることも確かめます。

```sh
python -m unittest discover -s foundation/tests/disk -v
```

## ビルドと実行

カーネルは §5.1 の配置（イメージ `0x10000`、カーネルスタック頂上 `0x100000`）でビルドします。各 `user.c` では `user_code_begin` を最初の関数、`user_code_end` を最後の関数にします。

```sh
python tools/llvm-src32/driver.py foundation/demo/user.c foundation/demo/main.c \
  foundation/kernel/sched.c foundation/kernel/proc.c foundation/kernel/trap.c foundation/kernel/ipc.c foundation/kernel/wax.c sdk/cpt32/src/pcg.c \
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

`tests/wax/main.c` は testbench 上の実行テストです。ローダの境界条件を細かく作るため、`tests/wax/build.py` が `*.asm` を `tools/asm` でアセンブルして v3 の WAX/WAL にし、`v2app.c` を `--protected` でビルドして、全部を `images.h` に書き出します。C から v3 を作る場合のテストは次の節です。

- `app`/`app2`（スロット5と6）：`test.math` と `test.base` を記述子経由で呼びます。モジュール間呼び出しでの R30 の保存と復元、コールバック、64-bit 引数の順序、2 つのスロットに置いた繰り返し import、オブジェクトの import、SYSCALL 後のレジスタを確認し、成功すると 42 で終了します。WAL はプロセスごとに別の基底に置かれ、呼び出し回数のカウンタも別々です。
- `poke_write`/`poke_exec`：RX への書き込みと RW の実行が、それぞれ `0x80050002` と `0x80050004` で fault になること。
- `v2app`：C で書いた PROTECTED WAX v2。
- 失敗する起動（依存がない、minor が古い、PROTECTED でない、壊れた画像、不正な優先度）が何も変えないこと。終了後に領域がゼロになり、同じスロットへ 2 回再起動できること。
- `spin` は CPU を使い続けます。`src32_testbench` は idle スレッドの `HALT` をプログラムの終了とみなすためです。

```sh
python foundation/tests/wax/build.py target/foundation-wax
python tools/llvm-src32/driver.py foundation/tests/wax/main.c foundation/kernel/sched.c \
  foundation/kernel/proc.c foundation/kernel/trap.c foundation/kernel/ipc.c foundation/kernel/wax.c \
  -I sdk/cpt32/include -I foundation/include -I target/foundation-wax \
  --origin 0x10000 --stack-top 0x100000 -O1 -o target/foundation-wax/test.wax
src32_testbench /absolute/path/target/foundation-wax/test.wax 42 --cpu pipeline --frames 90
```

`tools/llvm-src32/test_toolchain.py` の `test_foundation_wax_loader` が、`-O0`/`-O1`/`-O2` × Scalar/Cached/CachedRelaxed/Pipeline で実行します。記述子の origin をずらす、UGP を 0 にする、領域に全権限を与える、といった改変版ではこのテストが失敗することを確認しています。

### C から作る WAL のテスト

`tests/wal/` は、`tools/llvm-src32` の `--wal`、`--wax3`、`--import-wal`（[ツールチェーンの README](../tools/llvm-src32/README.md#wax-v3--wal)）で C から作った画像を、カーネルで実行します。`build.py` が `test.base`（WAL）、それを import する `test.math`（WAL、1.2）、両方を import する `test.app`（WAX v3、スロット5と6）を作り、`images.h` に書き出します。`app.c` は、WAL どうしの呼び出し、WAL 内部の関数ポインタ、WAL の静的データがプロセスごとに別であること、オブジェクトの import、文字列定数のアドレス、64-bit の引数と戻り値を確認し、すべて通れば 42 で終了します（起動コードが `proc_exit` を呼ぶ）。

```sh
python foundation/tests/wal/build.py target/foundation-wal
python tools/llvm-src32/driver.py foundation/tests/wal/main.c foundation/kernel/sched.c \
  foundation/kernel/proc.c foundation/kernel/trap.c foundation/kernel/ipc.c foundation/kernel/wax.c \
  -I sdk/cpt32/include -I foundation/include -I target/foundation-wal \
  --origin 0x10000 --stack-top 0x100000 -O1 -o target/foundation-wal/test.wax
src32_testbench /absolute/path/target/foundation-wal/test.wax 42 --cpu pipeline --frames 120
```

`tools/llvm-src32/test_toolchain.py` の `test_foundation_wal_from_c` が、画像を `-O0`/`-O1`/`-O2` で作り、Scalar と Pipeline で実行します。

## 注意と未決事項

- `src32_testbench` には UART がありません。そのため、そこで `debug_write` に 1 byte 以上を渡したり `kernel_panic` が出力したりすると、カーネル内でバスエラーになります。エミュレーター本体では UART に出力されます。
- RAM 容量の取得方法（§4.1）はまだ決めていません。スロットは既定の 16 MiB を前提にしています。
- 1プロセス1スレッドです。`thread_create`、`irq_bind`、`blk_io` はまだありません。
- `--protected` の C プログラムでは、アセンブラが `.ORG` で分けたブロックのうち、同じ権限で並ぶものの隙間（整列の埋め草）をゼロで埋めて 1 つにします（PROTECTED WAX は 7 ブロックまで）。ゼロ初期化のグローバルもイメージに含まれるので、シェルのイメージは約 230 KiB です。
- WAX の読み込み中は割り込み禁止です。`tests/wax` の起動 11 回（うち 5 回は失敗）は、-O0 の Scalar で約 16M サイクル（1 回あたり約 30 tick 分）かかります。-O2 では約 10M サイクルです。
- timer の ACK から IRQC の W1C までの間に次の tick が来ると、IRQC の edge が失われます。カーネルは W1C の後に timer の pending をもう一度確認して、取りこぼしを防いでいます。
