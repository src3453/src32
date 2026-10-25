# Foundation

CPT32 用の OS です。契約は [Foundation 仕様](../doc/spec_Foundation.md)、構成と開発順序は [実装案](../doc/foundation_os_plan.md) にあります。カーネルは C（[LLVM/SRC32 ツールチェーン](../tools/llvm-src32/README.md)）で書き、S モードで動きます。

CHIP-8 エミュレータの起動・キー配列・互換性は [CHIP-8 仕様と使い方](../doc/spec_Foundation_chip8.md) を参照してください。
外部 DOOMGeneric と手元の WAD から `doom` をビルド・起動する手順は [DOOMGeneric 移植](../doc/spec_Foundation_doom.md) を参照してください。
`/dev/sgc` は SGC の2Dコマンド FIFO を公開します（[API §9.3](../doc/spec_Foundation_API.md#93-sgc-2d-コマンド-fifo-devsgc)）。

`/dev/sgu` は SGU 2基・16チャンネルと共有 PCMRAM を1ストリームで公開します（[API §9.2](../doc/spec_Foundation_API.md#92-sgu-2基と共有-pcmram-devsgu)）。

`sgubplay /music/song.sgub` でディスク上のSGUBを再生できます。Esc/qで停止します。
曲の組み込み方と対応形式は [SGUBプレイヤー](../doc/spec_Foundation_sgubplay.md) を参照してください。

`wavplay /music/song.wav` はFIFO DACで8/16bit PCM WAVを再生します（SGU v2.1）。
ステレオは2chを左右に振り切り、モノラルは中央へ出力します。Esc/qで停止します。
対応形式・レート変換・組み込み方は [WAVプレイヤー](../doc/spec_Foundation_wavplay.md) を参照してください。

sol ゲストから libfnd を使う実行例は [`examples/foundation-sol`](../examples/foundation-sol/README.md) にあります。
`boot/build.py --root DIR` は追加ファイルをFS2ディスクに組み込みます。
起動時はIDC slot 0へそのディスクを接続してください。実fsサービスがマウントするため、
追加ファイルをfsの実行イメージへ埋め込む必要はありません。

## 表示モードの切替

シェルから次のコマンドを使えます。切替時は画面を消去して左上へ戻ります。

| コマンド | 表示 |
|---|---|
| `mode 40` | 40列×30行、8×8字形（320×240） |
| `mode 80` | 80列×30行、8×8字形（640×240、起動時の既定） |
| `mode 8x16` | 80列×30行、8×16字形（640×480） |
| `mode` | 現在の設定を表示 |

8×16は専用VRAMに設定済みの字形を使い、起動・表示切替・消去・スクロールで上書きしません。GUIの `config.yaml` の `pcg.font_8x16` またはlauncherの `PCG font 8x16` で4096 byteのrawフォントを指定してください。未設定で専用領域が空なら8×16の文字は表示されません。8×8用フォントは保持され、`mode 80`／`mode 40`で元へ戻せます。40列と8×16の組合せはPCGにないため、独立した3つの表示設定として切り替えます。LINE／RAW入力モードや現在の色は切替で変わりません。詳細は [API §8.1.2](../doc/spec_Foundation_API.md#812-コンソール表示モード) を参照してください。8×16表示では、JIS X 0208 の文字（日本語のかな・漢字・全角記号）を2セル幅で表示します。全角用フォントは最初に8×16へ切り替えたとき、コンソールがVRAM `0x200000` 以降へ自動で配置します（1 byte用の字形は上記の設定が必要です）。

## ANSI コンソール出力

標準出力（`fnd_con_write`／`fnd_write`）へ ANSI のサブセットを送れます。例えば C で `fnd_con_write("\033[31m赤色\033[0m\n");` と書くと赤の前景色に切り替えてから既定色へ戻します（PCG の表示できない文字は従来どおり `?` になります）。ASCII の動作例:

```c
fnd_con_write("\033[2J\033[H");          /* 全消去して左上へ */
fnd_con_write("\033[31;44mRED ON BLUE\033[0m\n");
fnd_con_write("\033[5;10Hposition\033[K"); /* 5行10桁、行末まで消去 */
```

カーソル移動・保存／復元、画面／行の消去、16色と高輝度化・反転、カーソル表示切替に対応します。色や解析状態は画面全体で共有されるため、終了前に `\033[0m`、カーソルを隠した場合は `\033[?25h` を送ってください。256色・RGB色、下線・点滅、スクロール領域、代替画面、端末応答は未対応です。詳細は [API §8.1.1](../doc/spec_Foundation_API.md#811-ansi-出力サブセット) を参照してください。

検証: `python -m unittest discover -s foundation/tests/console -v`。ホスト C コンパイラと、ゲスト検証には Clang・llvmlite・`src32_testbench` が必要です。`FND_MM=nommu` でも実行します。

## C版 TinyBASIC

シェルで `tinybasic` を起動すると対話モード、`tinybasic program.bas` ならファイルを読み込んで実行し、終了後にシェルへ戻ります。
`LOAD "path"` / `SAVE "path"`、標準入出力のリダイレクトにも対応します。
`python foundation/run.py --root examples/tinybasic-c/foundation-root` でデモ入りのディスクを起動し、`tinybasic demo.bas` を実行できます。
ビルド・言語・制限は [Foundation TinyBASIC仕様](../doc/spec_Foundation_tinybasic.md) を参照してください。

## フェーズ1: 最小カーネル

[`kernel_trap` トラップスタブ](../tools/llvm-src32/README.md)（フェーズ0）の上に、次のものを実装しています。

- **トラップ**（§3.4）: IRQ、SYSCALL、U モードの例外を `kernel_trap` で振り分けます。カーネルは割り込み禁止のまま、カーネルスタック1本で動きます。U モードで fault が起きたら、そのプロセスを終了させます。終了コードは CAUSE を i32 として扱った値で、fault の EPC も記録します。idle で例外が起きた場合、およびカーネル内で例外が起きた場合は `kernel_panic` になります。`kernel_panic` は PCG の文字面へ直接、原因・EPC・tick・実行中のプロセス・最後に保存したレジスタを描いてから UART へ出力し、停止します（[仕様 §3.4](../doc/spec_Foundation.md)）。
- **スケジューラ**（§10.1）: 優先度は DRIVER / SERVICE / APP / IDLE の4段で、同じ優先度の中は FIFO のラウンドロビンです。tick は PeC timer ch0 の 100 Hz（IRQ4、1 tick = 10 ms）、タイムスライス（クオンタム）は 1 tick（10 ms）です。より高い優先度のスレッドが起きたら、カーネルからユーザーへ戻る時点で切り替えます。idle は S モードのスレッドで、`HALT` を繰り返します。
- **プロセスと保護**（§5）: プロセスごとに連続したページの範囲（アリーナ）を割り当てます（V のない CPU。V のある CPU ではページ表）。別プロセスへ切り替えるときは、§5.3 の順（全 FLAGS=0 → START/END → FLAGS）で8領域を書き換えます。終了時は全スレッドを DEAD にし、付与した RAM をゼロにします。ユーザーポインタは §5.4 の手順で検証します。
- **システムコール**（§7, §9）: `sys_info`（sel 0〜3。sel 3 は Main RAM・VRAM・PCMRAM の容量）、`yield`、`sleep`（`FND_FOREVER` を含む）、`thread_exit`、`proc_exit`、`time_get`、`debug_write` を実装しています。ABI で定義済みでも未実装のものは `FND_E_UNSUPPORTED`、未定義の番号と context byte が 0 でない呼び出しは `FND_E_BAD_CALL` を返します。フェーズ2の部分は次の節です。

## フェーズ2: ハンドル、IPC、プロセスの起動

- **ハンドル**（§6）: PORT、CHANNEL、PROCESS、THREAD をプロセスごとの表（1〜63）で持ちます。`handle_close`、`handle_dup`、`XFER_HANDLE` による移動、終了時の一括クローズを実装しています。THREAD ハンドルは `wait` だけができ、dup は同じプロセス内に限ります。
- **IPC**（§8）: `port_create`、`chan_create`、`ipc_call`、`ipc_recv`、`ipc_reply`、`ipc_reply_recv`、`svc_register`、`svc_lookup`（badge の bit31 付き）、タイムアウト（§7.1）、最後のハンドルが閉じた CHANNEL の `CLIENT_GONE` 通知、相手がいなくなったときの `FND_E_PEER_GONE`。実装は `kernel/ipc.c` です。
- **プロセス**（§9）: `proc_spawn`（イメージと WAL の一覧 `libs` を staging に複製してから検証、ハンドルを子の 1 番から dup、PROCESS ハンドルを返す）、起動ブロック（§9.3）と `proc_startup`、`wait`、`proc_kill`、`mem_grow`（ヒープは最後の RW ブロックの末尾を 8 byte 境界に切り上げた位置から）、`sys_info` の `parent`。親が PROCESS ハンドルを閉じた終了済みのプロセスは、プロセス表から消えます。
- **同梱モジュール**（§11）: `boot/main.c` が FNDBOOT のモジュール表（console、disk、fs、init）を `fnd_proc_spawn_module` で起動します（argv = [名前]、cwd = `/`、役割なし）。
- `thread_create`、`thread_exit`（最後のスレッドの終了でプロセスも終了）、`irq_bind`、`irq_ack` を実装しています。IRQ capability を持つプロセスは IRQ 0、3、5〜12 を PORT に割り当てられます。`FND_NOTIFY_IRQ` を受信し、原因デバイスを ACK してから `fnd_sys_irq_ack` でマスクを解除し、デバイス状態を再確認します（仕様 §10.2）。`blk_io` は disk サービス向けに実装済みです。

IRQ の focused test は `python -m unittest discover -s foundation/tests/irq -v` です。
実際の VSync からの通知・ACK、複数 IRQ の通知保持、受信待ちの起床、権限と所有、
最後の PORT クローズ・プロセス終了時の解放を VM／slot、cached／pipeline で確認します。

`fnd_proc_create_builtin()` は、カーネルイメージにリンクしたユーザーコードからプロセスを作ります。WAX から作る方法は次の節を参照してください。このとき `[code_start, code_end)` が R/X、旧スロット上端の `ram_size` byte が R/W（スタックとデータ）になります。そのためユーザーコードでは、グローバル変数・文字列リテラル・範囲外の関数呼び出しを使えません（`examples/kernel-trap-c` と同じ制約）。システムコールは `<foundation/sys.h>` のマクロで呼ぶので、インライン展開されます。

| ファイル | 内容 |
|---|---|
| `include/foundation/abi.h`, `errors.h` | §9 の番号、§4 のエラーコード、優先度などの ABI 定数 |
| `include/foundation/sys.h` | U モード用のシステムコールマクロ |
| `include/foundation/kernel.h` | カーネルをリンクするイメージ（起動コード・デモ・テスト）向けの S モード API |
| `kernel/sched.c` | スレッド、優先度キュー、tick、sleep、idle |
| `kernel/proc.c` | プロセス、起動（`proc_spawn`、WAX の読み込み）、`mem_grow`、`sys_info`、終了処理 |
| `kernel/mm.c`, `mm.h` | メモリ管理層。nommu（ページアロケータ、プロセスごとのアリーナ、保護領域の切り替え、ポインタ検証）と仮想記憶（Extension V：フレーム表、ページ表、ユーザーメモリの複写）の 2 つのバックエンド（[仕様 §5.5](../doc/spec_Foundation.md)） |
| `kernel/trap.c` | `kernel_trap` / `kernel_panic`、IRQ、システムコール |
| `kernel/wax.c`, `wax.h` | WAX ローダの本体（検証、依存解決、WAL の配置、領域の計画、束縛） |
| `kernel/ipc.c`, `include/foundation/ipc.h` | ハンドル、IPC、サービス名、`wait`、`proc_kill`（§6〜§9） |
| `kernel/blk.c` | `blk_io`（§10.4）。IDC と DMAC ch0、IDC 割り込み（IRQ2）で要求を 1 つずつ FIFO で処理 |
| `kernel/fb.c`, `include/foundation/fb.h` | カーネル `/dev/fb0` ドライバとC helper（API §9.1）。ビットマップとCLUT |
| `kernel/fndboot.c` | FNDBOOT のモジュール表の検査と、同梱モジュールの起動（§11） |
| `include/foundation/api.h`, `fnd.h`, `lib/fnd/` | libfnd（[API 仕様 §11](../doc/spec_Foundation_API.md)）のうち、今あるもの。WAL「fnd」1.3（`/lib/fnd.1.wal`）としてビルドし、export は `lib/fnd/fnd.exports`。1.1はディレクトリ列挙・作成/削除・file delete/sync、1.2は `fnd_proc_fork` を追加、1.3は環境変数（`fnd_envc`、`fnd_env`、`fnd_exec_t.envp`）を追加 |
| `services/console/` | console サービス（API 仕様 §8、PCG と PeC キーボード） |
| `services/disk/` | disk サービス（`blk_io` を fs に IPC で渡す。プロトコルは `disk.h`） |
| `fs/` | fs サービス（FS2 と DSFS、VFS、API 仕様 §5〜§7 のプロトコル）。次の節 |
| `services/init/` | init（API 仕様 §12）。`/bin/shell.wax` を起動し、終了したら起動し直す |
| `shell/` | シェル（[シェル仕様](../doc/spec_Foundation_shell.md)）。ディスパッチャで、組み込みは `cd`、`set`、`exit`、`help` |
| `bin/` | 標準コマンド（シェル仕様 §10.3）のうち今あるもの：`clear`、`echo`、`free`、`mode`、`ps`、`pwd`、`top`、`uname` |
| `boot/` | カーネルの入口（`main.c`）、stage1（`stage1.a`）、FNDBOOT の packer（`fndboot.py`）、起動イメージとディスクイメージのビルドスクリプト（`build.py`） |
| `demo/` | 画面に各プロセスの状態を表示するデモ |
| `tests/phase1/` | フェーズ1の受け入れテスト |
| `tests/wax/` | WAX ローダのテスト（ホスト単体テストと testbench） |
| `tests/vm/` | 仮想記憶のテスト（同じアドレスのイメージの同時実行、ページフォルト、フレームの返却。testbench） |
| `tests/wal/` | C から作った WAL と WAX v3 を、カーネルで実行するテスト |
| `tests/shell/` | シェルのテスト（testbench にキー入力を送り、PCG の画面を比べる） |
| `tests/panic/` | `kernel_panic` の PCG 画面（カーネル内のバスエラーで原因・EPC・レジスタが描かれる） |
| `tests/disk/` | ディスク起動のテスト（BIOS → stage1 → FNDBOOT → シェル、stage1 とカーネルの拒否ケース） |

## WAX ローダ

`fnd_proc_spawn_wax(name, priority, image, size, libs, lib_count, grants, grant_count)` は、カーネルメモリ上の WAX を、アリーナに読み込んで起動します（固定アドレスの画像は `base_addr` を含む範囲を、再配置可能な画像は空きページを取り、再配置して読み込みます）（[仕様 §9.2・§9.5](../doc/spec_Foundation.md)）。

- 形式は PROTECTED の [WAX v2](../doc/spec_WAX.md) と [WAX/WAL v3](../doc/spec_WAX_v3.md) です。v3 では、依存する WAL を `libs` から選び（同じ ID と major の中で最大の minor）、アリーナ内に first fit で置き、import スロットに記述子（アドレスと origin）を書きます。
- 保護領域は region 0 がスタック、その後が全画像のブロックをアドレス順に並べたものです。隣接していて権限が同じブロックだけをまとめ、8 を超えると `FND_E_REGION_LIMIT` です。
- `grants` は MMIO の付与（§9.2 の `grants`、§10.3）で、画像の後の region になります（§5.3 の手順 4）。§10.3 の窓に収まらないものは `FND_E_ACCESS`、R（1）と RW（3）以外の perm は `FND_E_BAD_ARG` です。VRAM と PCMRAM は 1 プロセスにしか付与できず、動いている別のプロセスが持っていれば `FND_E_BUSY` です（§10.3 は誤りのコードを定めていないので、カーネルのコードから選んだ）。付与した範囲は §5.4 のポインタ検証の対象外で、終了時にゼロにもしません。
- 起動は原子的です。検証、依存解決、配置、領域とシンボルの確認がすべて通るまで、メモリにもプロセス表にも書きません。
- 入口の状態は §9.2 のとおりです（`USP = user_stack_top`、`UGP = base_addr`、他のレジスタは 0、`IE = 1`、スタックはゼロクリア）。終了時は、付与した範囲をすべてゼロにします。
- `wax.c` はカーネルの状態にもデバイスにも触れない C11 なので、ホストの C コンパイラでもビルドできます。

`proc_spawn` システムコールはこの関数の上にあり、§9.2 のとおりイメージと `libs` の WAL を先に staging（`0x00100000` 以降のカーネルヒープ）へ複製してから検証します。WAL は `/lib/<image_id>.<abi_major>.wal` に置き、起動する側が読んで `libs` で渡します（§9.5）。libfnd では `fnd_proc_exec` が依存表をたどって読みます。コピーはワード単位で、1 バイトずつだとシェルの読み込みだけで約 15 frame かかっていました。

## シェルと起動の構成

起動すると、カーネルが console、disk、fs、init を起動し、init が `/bin/shell.wax` を起動します（どれも再配置可能な WAX で、空いているアリーナに置かれます）。シェルは [シェル仕様](../doc/spec_Foundation_shell.md) 0.2 のディスパッチャで、`cd`、`set`、`exit`、`help` 以外のコマンドは `PATH`（既定 `/bin`）から `<name>.wax` を探し、`fnd_proc_exec` で起動して終了を待ちます。

- **libfnd の共有**：シェルと `/bin` のコマンドは、libfnd を WAL「fnd」1.2（`/lib/fnd.1.wal`）として import する WAX v3 です。コマンドは 1 つ約 9 KiB です（静的リンクでは約 38 KiB でした）。init と各サービスは、fs より先にカーネルが起動するので、libfnd を静的にリンクした PROTECTED WAX v2 です。
- **できること**：字句と構文（§3、§4）、変数（§5.1）、外部コマンドの解決と起動（§7。パス指定、`PATH`、大文字小文字を区別しない名前）、リダイレクトの解析と `2>&1`、行編集（§9.2）、エラーメッセージ（§11.1）、fault で終わったプロセスの報告。プロンプトは cwd と `> ` です。
- **まだできないこと**：ワイルドカードとスクリプト（`.fsh`）は `FND_E_UNSUPPORTED` です。`cat`、`cp`、`rm`、`mv`、`mkdir`、`rmdir`、`ls`、`hex` は `/bin` にあります（`cp` の同一ファイル検出はパス文字列の比較だけです）。Ctrl+C はシェルの専用スレッドが `CON_BREAK_WAIT` で受け取り、実行中の外部プログラムを `proc_kill` で止めます（`$?` は `FND_E_KILLED`、STDERR に `^C`）。行編集中は保留中の `CON_READ_KEY` が `FND_E_INTERRUPTED` になり、入力中の行を捨てます。組み込みコマンドの中断とスクリプトの入れ子中断は、スクリプト（KAN-19）とともに未実装です。
- **ファイル**：`/bin` と `/lib` はブートディスク（FS2）上のファイルで、fs サービスが `/` として見せます（次の節）。`boot/build.py` が `OUT/root` に書き出し、それをディスクイメージに入れます。
- **一覧表示**：`ls [path...]` は名前をタブ区切りで表示します。詳細を 1 件 1 行で見るには `ls -l [path...]` を使います。
- **起動イメージ**：`boot/main.c` は FNDBOOT（§11）のモジュール表（console、disk、fs、init の順）を spawn します。console の MMIO の付与と各モジュールの `caps` はモジュール表に書いてあります。ディスク起動では stage1 が読み込んだ FNDBOOT を使い、`foundation.wax` では `boot/build.py` がカーネルを含まない FNDBOOT を `0x00200000` の block として足します（stage1 が置く場所と同じ。次の節）。シェルの再起動は init が行うので、モジュールを起動した後は staging 領域を `proc_spawn` が使います。
- **console**：`services/console` が PCG（既定はSCREEN_MODE 1、80×30、8×8。`mode`で切替可能）と PeC のキーボードを MMIO で扱います（API 仕様 §8）。console は引き続きポーリングを使い、キーは IPC の合間と 1 tick（10 ms）ごとに FIFO から読み、256 キーまで先行入力をためます。`CON_READ_KEY` の `NOWAIT`（`fnd_con_pollkey`、`top` が使う）は待たずに答えます。`CON_SET_COLOR` は ANSI 順の16色に対応します。
- `uname`、`free`、`ps`、`top` と起動時の表示は `sys_info` を使います。`ram_bytes` は BMC が報告する Main RAM の容量（§5.1、報告がなければ 16 MiB）、`free_bytes` は仮想記憶では未使用の物理フレームの合計、nommu ではページプールの空きページの合計です（アリーナの末尾の余り＝ヒープの予備は、プロセスが持っているので含みません）。`free` は sel 3 で VRAM と PCMRAM の容量も表示、`boot_drive` はディスク起動では stage1 から渡された drive、`foundation.wax` では 0 です。

```sh
python foundation/run.py
```

`/bin/top` は CPU 使用率（user / kernel 概算 / idle）、メモリ容量と CPU 使用率順のプロセス一覧を 1 秒ごとに更新します。`q` または Ctrl+C で戻ります。`top -n 1` は 1 回分を表示して終了します。kernel の値は C ハンドラの計測で、context save/restore を含む厳密な OS オーバーヘッドではありません。

リポジトリルートで実行してください。スクリプトを絶対パスで指定すれば、別の作業ディレクトリからも使えます。Python と llvmlite、Clang、Rust/Cargo が必要です（[ツールチェーンの準備](../tools/llvm-src32/README.md#必要なツール)）。`--clang PATH` で Clang を指定でき、`--build-only` はビルドと設定生成だけを行います。`-j N` は SRC32 側の Clang 並列数で、既定はマシンの物理コア数（SMT を除く）です。

`--arg` 以降の引数は、そのままエミュレーターへ渡します。`run.py` のオプションは `--arg` より前に指定してください。空白を含む値は通常のシェル引数と同様に引用符で囲みます。

外部でビルドしたコマンドは `--bin-dir DIR` で毎回ディスクへ追加できます。

```sh
python foundation/run.py --bin-dir target/foundation-doom --bin-dir target/busybox
```

指定ディレクトリ直下の `.wax`（拡張子の大文字小文字は不問）を `/bin/<ファイル名>` に実行可能な mode `0x003D` で取り込みます。サブディレクトリ、WAL、WAD などは対象外です。存在しないディレクトリや WAX のないディレクトリは、コンパイル開始前にエラーになります。標準ファイル、`--root`、`--bin-dir` の順に適用し、同名のファイルは後の指定を優先します。元の WAX はビルド開始時に読み取るため、ビルドで生成先が上書きされる場合も取り込めます。標準コマンドも上書きできるので、専用の外部コマンド出力ディレクトリを指定してください。

RAM は `--ram-mib N`（既定 16、1〜256）、ディスクは `--disk-blocks N`（512-byte block、既定 16384 = 8 MiB）で設定できます。例えば DOOM 用の WAX と WAD を追加する場合：

```powershell
New-Item -ItemType Directory -Force target/doom-root | Out-Null
Copy-Item C:/Users/y2k34/Downloads/doom/DOOM.WAD target/doom-root/doom.wad
python foundation/run.py --bin-dir target/foundation-doom --root target/doom-root --ram-mib 64 --disk-blocks 131072
```

Foundation のシェルで `doom -iwad /doom.wad` を実行します。外部 WAX 自体のビルドは行わないため、先に [DOOMGeneric のビルド](../doc/spec_Foundation_doom.md)などで用意してください。追加 WAL が必要なコマンドでは `--root DIR` の `DIR/lib/` も使います。`boot/build.py OUT` にも同じ `--bin-dir` を指定できます。

```sh
python foundation/run.py --disk --arg --debug-gui --start-paused --cpu pipeline
```

このコマンドはシェルとカーネルを `target/foundation-boot/foundation.wax` にビルドし、release エミュレーターをビルドして起動します。作業用一時ファイルも同ディレクトリ内に置きます。`target/foundation-boot/run/config.yaml` に指定した RAM 容量（既定 16 MiB）と付属の `assets/UNSCII.F08` フォントを設定し、そのディレクトリをエミュレーターの作業ディレクトリにします。普段使うルートの `config.yaml` は変更しません。

手動で起動する場合は `python foundation/boot/build.py target/foundation-boot` で画像を作り、BIOS 未設定の作業ディレクトリから画像の絶対パスをエミュレーターへ渡してください。`bios.image` があると WAX の入口より BIOS が優先されます。PCG の文字を表示するには、作業ディレクトリの `config.yaml` に `pcg.font`（例：`assets/UNSCII.F08` の絶対パス）が必要です。

`tests/shell/test_shell.py` は、`--stop-ticks 800` でビルドした起動イメージを `src32_testbench` で 660 frame 動かします（ディスクからの起動に約 115 frame かかるため）。`--type FRAME=TEXT` でその frame にキーを送り（`\n` Enter、`\b` Backspace、`\L` `\R` `\U` `\D` 矢印、`\H` Home、`\E` End、`\X` Delete、`\C` Ctrl+C、`\\` バックスラッシュ）、`--dump-pcg` で最後の画面（SCROLL_Y を反映した 30 行）を出力させて比べます。GUI は Xvfb の下では黒い画面になるので、画面の確認はこの方法で行います。

```sh
python -m unittest discover -s foundation/tests/shell -v
```

通常の機能検証は Cached CPU で実行します。再起動後の `ps` と CPU 使用率の snapshot は Pipeline を使い、スケジュールとサイクルに依存する経路も毎回検証します。
全件を Pipeline で調べる場合は `FOUNDATION_TEST_CPU=pipeline` を設定してください（PowerShell: `$env:FOUNDATION_TEST_CPU='pipeline'`）。各ケースは起動から実行し、ディスクを毎回コピーするため、書き込みや DIRTY 状態はほかのケースに漏れません。
再起動テストの shell 状態は RUN/BLOCK の両方が正当です。子の生成後、`fnd__exec` がイメージを解放している間に time slice が切れると、子の `ps` は親が `wait` でブロックする前に実行されます。新しい PID と親子関係、終了した shell の回収は状態とは別に検証しています（KAN-37）。

## ディスク起動

`python foundation/boot/build.py OUT` は `foundation.wax` のほかに、BIOS から起動できる FS2 ディスク `OUT/foundation.img` も作ります。起動の流れは [仕様 §11](../doc/spec_Foundation.md)、[FS2 §3](../doc/spec_FS2.md)、[BIOS §3](../doc/spec_BIOS.md) のとおりです。

1. BIOS が IDC dd0〜dd3 のうち署名のある最初の drive の LBA 0（`boot/stage1.a`、512 bytes）を `0x0` に読み、R1 = drive で分岐します。
2. stage1 は superblock（LBA 1）を検査し（magic、version、flags = 0 なので DIRTY = 0、`total_blocks` ≤ 容量、`boot_start` = 2、`bitmap_start` = 2 + `boot_blocks` ≤ `data_start` ≤ `total_blocks`、0 < `boot_size` ≤ 2 MiB、予約領域に収まること）、予約ブート領域から FNDBOOT を `0x00200000` に読みます。FNDBOOT のヘッダを検査してカーネルを `0x00010000` へコピーし、R1 = drive、R2 = `0x00200000` で `kernel_entry` へ分岐します。失敗すると R1 = 1（IDENTIFY か superblock の読み込み）、2（superblock）、3（FNDBOOT）で HALT します。画面には何も出ません（510 bytes に収めるため）。
3. カーネル（`main(drive, fndboot)`）は `boot_drive` を記録し（`sys_info`）、`fnd_boot_check` でモジュール表全体を検査してから、表の順に `fnd_boot_spawn` します。表が不正なら main が `FND_E_FORMAT` を返し、カーネルはその値を R1 に入れて HALT します。

ディスクの中身は次のとおりです。既定は 8 MiB（16384 blocks、inode 256）で、予約ブート領域は stage1 が受け付ける最大の 2 MiB（4096 blocks）です。FNDBOOT が大きくなっても再 format は要りません。

| 場所 | 中身 |
|---|---|
| block 0 | `stage1.bin`（署名 `55 AA` 付き） |
| 予約ブート領域 | `fndboot.bin`：ヘッダ、モジュール表（console、disk、fs、init。優先度 SERVICE、console には MMIO 付与）、`kernel.bin`（`0x10000` の raw image）、各モジュールのイメージ |
| `/bin/*.wax`、`/lib/fnd.1.wal` | シェル、標準コマンド、libfnd（mode `0x003D`）。`OUT/root` と同じもの。fs サービスがディスクから読んで返します |
| `--root DIR` | ホストの `DIR` の中身をそのままルートへ置きます（例：`DIR/bin/*.wax`、`DIR/etc/startup`）。`/bin` の下は mode `0x003D` です |
| `--bin-dir DIR` | ホストの `DIR` 直下の `.wax` を `/bin` に追加します。複数指定でき、`--root` より優先します |

```sh
python foundation/run.py --disk
```

で、BIOS（`assets/bios.rom`）と dd0 の HDD（書き込み可）を設定してエミュレーターを起動します。プログラムには先頭 word が 0 の 4 bytes を渡すので、BIOS は RAM 起動を飛ばしてディスクから起動します。POST の表示は120 frame保持します。全マウント検査を行っていた時点では、その後HDDで約115 frame、FDD（`type: "fdd"`、62,500 B/s）で約6秒かかっていました。現在は下記の暫定対策で全走査を省略します。ディスクの中身は `tools/fs2/fs2.py`（[README](../tools/fs2/README.md)）で見たり変えたりできます。

```sh
python tools/fs2/fs2.py ls target/foundation-boot/foundation.img /bin
python tools/fs2/fs2.py check target/foundation-boot/foundation.img
```

`tests/disk/test_disk_boot.py` は `--stop-ticks 600` でビルドしたディスクを、BIOS と `src32_testbench` で起動します。dd0 を空にして dd1 から起動し（stage1 が R1 の drive を使うことの確認）、`uname` を打って画面を比べます。superblock の DIRTY と配置、FNDBOOT の magic と size、モジュール表の予約欄を壊したディスクでは、それぞれ決まった R1 で止まることも確かめます。

```sh
python -m unittest discover -s foundation/tests/disk -v
```

## ビルドと実行

カーネルは §5.1 の配置（イメージ `0x10000`、カーネルスタック頂上 `0x100000`）でビルドします。各 `user.c` では `user_code_begin` を最初の関数、`user_code_end` を最後の関数にします。

```sh
python tools/llvm-src32/driver.py foundation/demo/user.c foundation/demo/main.c \
  foundation/kernel/sched.c foundation/kernel/proc.c foundation/kernel/mm.c foundation/kernel/trap.c foundation/kernel/ipc.c foundation/kernel/fb.c foundation/kernel/sgu.c foundation/kernel/sgc.c foundation/kernel/blk.c foundation/kernel/wax.c sdk/cpt32/src/pcg.c \
  -I sdk/cpt32/include -I foundation/include --origin 0x10000 --stack-top 0x100000 -O1 \
  -o target/foundation_demo.wax
cargo run --release --bin cpt32 -- target/foundation_demo.wax
```

デモでは、yield しない3つのカウンタが 1 tick（10 ms）ずつ交互に進みます。`crash` は2〜3秒後にカーネル領域へ書き込み、`exit 80050002`（保護違反）で終了します。残りのプロセスはそのまま動き続けます。SERVICE 優先度の `ticker` は 25 tick（250 ms）ごとに起き、そのたびにカウンタを割り込んで実行されます。

CMake の場合（`$python` と `$ninja` は [Hello C++](../examples/hello-cpp/README.md) と同じ）:

```sh
cmake -S foundation -B target/foundation -G Ninja -DSRC32_PYTHON_EXECUTABLE=$python -DSRC32_CLANG=/path/to/clang
cmake --build target/foundation
```

これで `foundation_demo.wax` と `foundation_phase1_test.wax` ができます。

## 受け入れテスト

IPCの性能測定は [tests/perf/README.md](tests/perf/README.md) を参照してください。VM/slot、メッセージサイズ、整列、`reply_recv` を実際のUモード間通信で比較し、[基準値と改善候補](../doc/foundation_ipc_performance.md) を記録しています。

カーネルは256 bytes以上の重ならないMain RAMコピーに同期DMA（専用ch1）を使用します。IPCのペイロードとディスク用bounce bufferのコピーが対象に含まれ、短いIPCやconsole内の文字描画はCPU処理です。[同期DMAの契約](../doc/spec_Foundation.md#541-カーネル内の同期dmaコピー)。境界・チャネル共存の確認は `python -m unittest discover -s foundation/tests/perf -p test_copy.py -v`、console/fsの大容量I/O確認は `python -m unittest foundation.tests.console.test_console_guest.ConsoleGuestTests.test_bulk_console_and_disk_io -v`（`FND_MM=nommu`でも実行）です。

`tests/phase1` は、[実装案 §8](../doc/foundation_os_plan.md) のフェーズ1の条件を確認します。tick フックの中で検査し、成功すると R1=42、失敗すると `0x100 + 失敗数` で停止します。

- yield しない U モードのカウンタ A・B が両方とも進むこと。
- 別プロセスが A の領域に書き込もうとすると、保護違反（`0x80050002`）でそのプロセスだけが終了すること。A への書き込みは届かず、終了したプロセスの RAM はゼロになり、A と B はその後も進むこと。
- SERVICE プロセスについて、次のことを確認します。
  - `sleep(7)` が 7〜8 tick で戻ること（APP を割り込んで実行される）。
  - 不正なポインタは `FND_E_ACCESS` になること。
  - 未定義の番号や context 付きの呼び出しは `FND_E_BAD_CALL`、未実装の呼び出しは `FND_E_UNSUPPORTED` になること。
  - `time_get` で tick 数を取得できること。
  - `proc_exit` の終了コードが記録されること。
- SERVICE プロセスの終了後は、1 tick（10 ms）のスライスだけで A と B が毎 tick 交互に進むこと。独立した PeC ch1（48 MHz）の経過時間でも tick 間隔を確認します。

```sh
src32_testbench /absolute/path/foundation_phase1_test.wax 42 --cpu pipeline --frames 90
```

`tools/llvm-src32/test_toolchain.py` の `test_foundation_phase1_kernel` が、`-O0`/`-O1`/`-O2` × Scalar/Cached/CachedRelaxed/Pipeline で実行します。スライスによる切り替えや保護領域の切り替えを無効にした改変版では、このテストが失敗することも確認しています。

### WAX ローダのテスト

`tests/wax/test_wax_loader.py` は、`wax.c` をホストの C コンパイラ（`CC`）で共有ライブラリにして、[WAX v3 §8](../doc/spec_WAX_v3.md) の受理・拒否ケースを確認します。v3 の画像は `tools/wax/wax3.py` で組み立てます。

```sh
python -m unittest discover -s foundation/tests/wax -v
```

`tests/wax/main.c` は testbench 上の実行テストです。ローダの境界条件を細かく作るため、`tests/wax/build.py` が `*.asm` を `tools/asm` でアセンブルして v3 の WAX/WAL にし、`v2app.c` を `--protected` でビルドして、全部を `images.h` に書き出します。C から v3 を作る場合のテストは次の節です。

- `app`/`app2`（固定アドレス 5 と 6 のスロット相当）：`test.math` と `test.base` を記述子経由で呼びます。モジュール間呼び出しでの R30 の保存と復元、コールバック、64-bit 引数の順序、2 つのスロットに置いた繰り返し import、オブジェクトの import、SYSCALL 後のレジスタを確認し、成功すると 42 で終了します。WAL はプロセスごとに別の基底に置かれ、呼び出し回数のカウンタも別々です。
- `poke_write`/`poke_exec`：RX への書き込みと RW の実行が、それぞれ `0x80050002` と `0x80050004` で fault になること。
- `v2app`：C で書いた PROTECTED WAX v2。
- 失敗する起動（依存がない、minor が古い、PROTECTED でない、壊れた画像、不正な優先度）が何も変えないこと。終了後に領域がゼロになり、同じ範囲へ 2 回再起動できること。再配置可能な画像（`pie`）を 3 つ同時に起動して別々のアリーナに置かれ、終了後にアリーナがゼロになりプールへ戻ること。
- `spin` は CPU を使い続けます。`src32_testbench` は idle スレッドの `HALT` をプログラムの終了とみなすためです。

```sh
python foundation/tests/wax/build.py target/foundation-wax
python tools/llvm-src32/driver.py foundation/tests/wax/main.c foundation/kernel/sched.c \
  foundation/kernel/proc.c foundation/kernel/mm.c foundation/kernel/trap.c foundation/kernel/ipc.c foundation/kernel/fb.c foundation/kernel/sgu.c foundation/kernel/sgc.c foundation/kernel/blk.c foundation/kernel/wax.c \
  -I sdk/cpt32/include -I foundation/include -I target/foundation-wax \
  --origin 0x10000 --stack-top 0x100000 -D FND_MM_NOMMU_ONLY -O1 -o target/foundation-wax/test.wax
src32_testbench /absolute/path/target/foundation-wax/test.wax 42 --cpu pipeline --frames 90
```

`tools/llvm-src32/test_toolchain.py` の `test_foundation_wax_loader` が、`-O0`/`-O1`/`-O2` × Scalar/Cached/CachedRelaxed/Pipeline で実行します。記述子の origin をずらす、UGP を 0 にする、領域に全権限を与える、といった改変版ではこのテストが失敗することを確認しています。

### C から作る WAL のテスト

外部 C ライブラリを WAL 化する例は [LZMA SDK デモ](../examples/lzma-wal/README.md) を参照してください。
`liblzma.wal` の圧縮・展開 API、export 定義、WAX 側の import、ディスクへの配置を説明しています。
`liblzma.wal` は `/lib/lzma.1.wal` として既定のディスクイメージに含まれます（`boot/build.py` が `-O` の指定に従って作ります）。
`libsgc`（SGC の2Dコマンド構築と FIFO 投入、[API §9.3](../doc/spec_Foundation_API.md#93-sgc-2d-コマンド-fifo-devsgc)）は `/lib/sgc.1.wal` として同様に含まれ、`fnd` WAL を import します。`chip8` が利用しています。

### 圧縮ライブラリ共通 API「conde」

圧縮ライブラリ（lzma、rle、今後追加するもの）は conde（compression-decompression）に従います。規約は [`include/foundation/conde.h`](include/foundation/conde.h) に書いてあります。関数は `cpt_<algo>_compress`／`cpt_<algo>_decompress`／`cpt_<algo>_compress_bound` の形に揃え、戻り値は `CONDE_*`、`*dst_size` は容量を入力して実長を返し、作業領域は呼出側が渡します（不要なアルゴリズムは NULL／0 でよい）。WAL 間では関数ポインタを渡せないため、共通化は名前・シグネチャ・コードで行います。

`librle.wal`（`lib/rle`、image ID `rle`、ABI 1.0）は PackBits 形式の RLE で、`/lib/rle.1.wal` として既定のディスクに入ります。heap・libfnd・作業領域を使わず、4 byte 単位でラン検出・コピー・埋め込みを行います。テストは `python3 -m unittest foundation.tests.rle.test_rle`（Clang と `src32_testbench` が必要。後者がなければ Foundation 上のテストは skip）です。

### 文字コード変換 libcode

`libcode.wal`（`lib/code`、image ID `code`、ABI 1.0、`/lib/code.1.wal`）は UTF-8（BOM あり／なし）、UTF-16LE／BE、UTF-32LE／BE を変換します。UTF-32BE を基準の形式とし、`cpt_code_<enc>_to_utf32be` と `cpt_code_utf32be_to_<enc>`（`<enc>` は `utf8`、`utf8bom`、`utf16le`、`utf16be`、`utf32le`）の対で変換します。他の符号化どうしは UTF-32BE を経由します。引数・戻り値・`*dst_size` は conde と同じ形（`flags` を追加）で、規約は [`include/foundation/code.h`](include/foundation/code.h) にあります。

- 不正な列（過長形式、サロゲート、U+10FFFF 超、対にならないサロゲート）は `CONDE_DATA`、末尾で切れた列は `CONDE_INPUT_EOF`。`CODE_REPLACE` を付けると不正な部分（maximal subpart）ごとに U+FFFD を書いて続けます。
- BOM: `utf8bom` への変換は BOM を必ず付け、からの変換は先頭の BOM を読み飛ばします。他の符号化には `CODE_ADD_BOM`／`CODE_STRIP_BOM`。BOM から符号化を調べるのは `cpt_code_detect`（`FF FE 00 00` は UTF-32LE）。
- 出力の見積もりは `cpt_code_bound`。heap・libfnd・作業領域は使いません。
- テストは `python3 -m unittest foundation.tests.code.test_code`（ホストは Clang、Foundation 上は `src32_testbench` も必要）です。

`tests/wal/` は、`tools/llvm-src32` の `--wal`、`--wax3`、`--import-wal`（[ツールチェーンの README](../tools/llvm-src32/README.md#wax-v3--wal)）で C から作った画像を、カーネルで実行します。`build.py` が `test.base`（WAL）、それを import する `test.math`（WAL、1.2）、両方を import する `test.app`（WAX v3、固定アドレス 5 と 6 のスロット相当）を作り、`images.h` に書き出します。`app.c` は、WAL どうしの呼び出し、WAL 内部の関数ポインタ、WAL の静的データがプロセスごとに別であること、オブジェクトの import、文字列定数のアドレス、64-bit の引数と戻り値を確認し、すべて通れば 42 で終了します（起動コードが `proc_exit` を呼ぶ）。

```sh
python foundation/tests/wal/build.py target/foundation-wal
python tools/llvm-src32/driver.py foundation/tests/wal/main.c foundation/kernel/sched.c \
  foundation/kernel/proc.c foundation/kernel/mm.c foundation/kernel/trap.c foundation/kernel/ipc.c foundation/kernel/fb.c foundation/kernel/sgu.c foundation/kernel/sgc.c foundation/kernel/blk.c foundation/kernel/wax.c \
  -I sdk/cpt32/include -I foundation/include -I target/foundation-wal \
  --origin 0x10000 --stack-top 0x100000 -D FND_MM_NOMMU_ONLY -O1 -o target/foundation-wal/test.wax
src32_testbench /absolute/path/target/foundation-wal/test.wax 42 --cpu pipeline --frames 120
```

`tools/llvm-src32/test_toolchain.py` の `test_foundation_wal_from_c` が、画像を `-O0`/`-O1`/`-O2` で作り、Scalar と Pipeline で実行します。

### 仮想記憶のテスト

カーネルは `CPUID` で Extension V を見つけると、nommu のアリーナの代わりにページ表を使います（[仕様 §5.5](../doc/spec_Foundation.md)、設計は [foundation_vm_plan.md](../doc/foundation_vm_plan.md)）。V のない CPU を想定して nommu（アリーナ方式）で動かすには、カーネルを `-D FND_MM_NOMMU_ONLY` でビルドします（`boot/build.py` は環境変数 `FND_MM=nommu`）。同じソースの 2 つのバックエンドを、次のように確かめます。

- `tests/phase1`、`tests/wax`、`tests/wal` は物理メモリの中身を直接見るので、`FND_MM_NOMMU_ONLY` でビルドします。
- `tests/shell`、`tests/disk`、`tests/lzma` など、起動イメージを丸ごと動かすテストは、どちらのモードでも通ります。`tests/shell` は実際に使う testbench と CPU エンジンで `cpu_features.a` の CPUID を実行し、free の空き容量は、どちらのモードでも値を N に正規化して比べます。`FND_MM=nommu` は CPU の V の有無にかかわらず nommu です。

`tests/shell` は `SRC32_TESTBENCH` があればその実行ファイル、なければ `target/release`、`target/debug` の順で testbench を探します。古い release 版が残っていると新しい debug 版より優先されます。仮想記憶を検証するときは `cargo build --release --bin src32_testbench` で更新するか、`SRC32_TESTBENCH` に V 対応のビルドの絶対パスを指定してください。

```sh
python -m unittest discover -s foundation/tests/shell          # 仮想記憶（V のある testbench）
FND_MM=nommu python -m unittest discover -s foundation/tests/shell  # nommu（アリーナ）
```

`tests/vm/main.c` は testbench 上の実行テストで、`tests/wax/build.py` の画像を使います。同じアドレスにリンクした `app` を 2 つ同時に起動してどちらも 42 で終了すること（nommu のアリーナでは、固定アドレスの画像どうしは `FND_E_ADDRESS_CONFLICT`。再配置可能な画像は別の場所に置かれます）、RX への書き込みと RW の実行がページフォルト（`0x8006000A`、`0x8006000C`）で終わること、拒否された起動と終了したプロセスがフレームを一つも残さないこと（未使用フレームの合計が起動前と一致）を確認します。`tools/llvm-src32/test_toolchain.py` の `test_foundation_vm_kernel` が `-O0`/`-O1`/`-O2` × 4 種の CPU で実行します。

## fs サービス

`fs/`は FS2（[仕様](../doc/spec_FS2.md)）を `/` として見せる fs サービスです。disk サービス（`services/disk/`）の IPC を通して、カーネルの `blk_io`（IDC + DMAC）でディスクを読み書きします。

- **VFS**：`/` が FS2、`/tmp` と `/dev`（`/dev/con`、`/dev/null`、`/dev/fb0`、`/dev/sgu`、`/dev/sgc`、`/dev/trng`、`/dev/prng`、`/dev/uart0`）がメモリ上のノードです。`/dd/N` は dsfs が受け持ちます。`/dev` のノードでは TRUNC と APPEND を無視するので、`> /dev/null` が使えます。
- **VDP bitmap**：`/dev/fb0` はカーネルドライバです。通常のopen/read/writeと `<foundation/fb.h>` で、320×240のINDEX8／RGB555／RGB888と全256色のCLUTを操作できます。openは排他で、最後のclose・プロセス終了時に表示設定・CLUT・重なったPCG文字領域を復元します。[実行例](../examples/foundation-fb/README.md)、[BMP 表示 `bmpview`](../examples/foundation-bmp/README.md)、[API仕様 §9.1](../doc/spec_Foundation_API.md#91-カーネル-vdp-ビットマップドライバ-devfb0)。
- **マウント**：KAN-38 の暫定対策で、fsサービスの全チェーン・ディレクトリ・参照整合性検査を省略します。署名・レイアウト・DIRTY拒否とbitmap/inodeの基本検査は残します。clean媒体の破損を見逃すため、事前に `python tools/fs2/fs2.py check IMAGE` で検査してください。`foundation/boot/build.py` の `FS2_FULL_MOUNT_CHECK` を1に戻して再ビルドすると、§10の完全検査を復元できます（[仕様の暫定例外](../doc/spec_FS2.md#101-foundation-の一時的な起動対策kan-38)）。disk サービスは128 sectorの先読み窓を4つ（LRU、計256 KiB）、fsは8 sectorの先読みと32ラインのキャッシュを使います。書込範囲と重なるdiskの全窓は書込前に無効化し、IDENTIFYでは該当driveの全窓、read/writeエラーでは全窓を無効化します。性能は[起動性能の計測](../doc/foundation_spawn_profile.md)を参照してください。キャッシュ単体の検証は `python -m unittest discover -s foundation/tests/disk_cache -v` で実行できます。
- **DIRTY**：最初の書き込みで DIRTY を立てます。シャットダウン時にはアンマウントしないので、書き込んだ後は fsck が要ります（ホスト側の `tools/fs2` は `allow_dirty` で読めます）。
- **起動時間**：完全検査時はPipelineの直接WAX起動＋IDC上のFS2で初回プロンプトまで約100 frameでした。この値はKAN-38の全走査省略前の値です。[KAN-36 の計測](../doc/kan36_performance.md)を参照してください。

## 注意と未決事項

- `src32_testbench` には UART がありません。そのため、そこで `debug_write` に 1 byte 以上を渡したり `kernel_panic` が UART へ出力したりすると、カーネル内でバスエラーになります（`kernel_panic` は PCG へ先に描くので画面には残り、再入は黙って停止します）。エミュレーター本体では UART に出力されます。
- RAM 容量の取得方法（§4.1）はまだ決めていません。nommu のページプールは [0x400000, RAM 末尾)、最大 256 MiB です。
- 仮想記憶では、権限はページ単位です。Foundation のイメージは `--page-align` でコード（RX）とデータ（RW）を別ページに置くので W^X です（`.rodata` は RW データの中で、R 専用ではありません）。MMIO の付与はページ単位で、窓の検査は byte 単位に加えてページ範囲が届く先も検査しますが、同じページにある別のレジスタには触れます（[仕様 §5.5](../doc/spec_Foundation.md)）。
- プロセスは（仮想記憶でも nommu でも）10 個、スレッドは 16 個までです。スロットにもアリーナにも縛られないので、`FND_MAX_PROCS` を増やせます。
- スレッドは全体で `FND_MAX_THREADS`（16）個です。IRQ の割り当ては PORT の最後のクローズまたは所有プロセスの終了で解放されます。
- `--protected` の C プログラムでは、アセンブラが `.ORG` で分けたブロックのうち、同じ権限で並ぶものの隙間（整列の埋め草）をゼロで埋めて 1 つにします（PROTECTED WAX は 7 ブロックまで）。ゼロ初期化のグローバルもイメージに含まれるので、シェルのイメージは約 230 KiB です。
- WAX の読み込み中は割り込み禁止です。`tests/wax` の起動 11 回（うち 5 回は失敗）は、-O0 の Scalar で約 16M サイクル（1 回あたり約 30 tick 分）かかります。-O2 では約 10M サイクルです。
- timer の ACK から IRQC の W1C までの間に次の tick が来ると、IRQC の edge が失われます。カーネルは W1C の後に timer の pending をもう一度確認して、取りこぼしを防いでいます。


### PeC 乱数と UART

`/dev/trng` は READ 専用で、ハードウェアの READY が落ちている間は呼び出したスレッドをブロックします。`/dev/prng` は READ 専用のノンブロッキング GFSR 乱数です（暗号用途向けではありません）。`/dev/uart0` はホスト標準入出力へつながるバイトストリームで、読み出しは受信待ち、書き込みはデータをそのまま送信します。通常の `fnd_file_open`／`fnd_read`／`fnd_write` で利用できます。複数 open は同じハードウェアを共有します。乱数列の seed 設定 API は公開しません。[API §9.4](../doc/spec_Foundation_API.md#94-pec-乱数uart-devtrngdevprngdevuart0)。

検証: `python -m unittest discover -s foundation/tests/pec -v`。ゲスト検証は `FND_MM=nommu` でも実行してください。testbench には UART がないため、UART の正長入出力と TRNG の未準備からの復帰は、実ドライバと IPC ディスパッチを制御可能な MMIO に接続するホストテストで確認します。ゲスト側では RNG の 4096 bytes をまたぐ read、短い末尾、VFS とハンドル転送、権限、stat、複数 open を確認します。

### COW と fork

VM バックエンドでは `fnd_sys_proc_fork()`（`<foundation/sys.h>`）または libfnd 1.2 の `fnd_proc_fork()` を使えます。親は子の PROCESS ハンドル、子は 0 を受け取り、同じ位置から実行を再開します。親は `fnd_sys_wait()` と `fnd_sys_handle_close()`、または `fnd_proc_wait()` で子を回収します。RW メモリは書き込み時に 4 KiB ごとに私有化されます。ファイル位置や fs セッションなどのサービス側状態は共有です。生存スレッド 1 本が必要で、THREAD ハンドルや MMIO 付与のあるプロセスは複製できません。nommu（V のない CPU）では `FND_E_UNSUPPORTED` です。詳細は [カーネル仕様 §9.6](../doc/spec_Foundation.md) を参照してください。

回帰テストは `python -m unittest discover -s tools/llvm-src32 -p test_toolchain.py -k test_foundation_fork -v`。`SRC32_TESTBENCH` に release 版の絶対パスを渡すと実行時間を抑えられます。VM と nommu、O0/O1/O2、scalar/cached/cached-relaxed/pipeline を検証します。
