# Foundation カーネル ABI 仕様 v1

Revision 1.0、2026-10-22。Status: **仕様確定**。§3 のトラップスタブは `tools/llvm-src32` に実装済み（フェーズ 0）。§13 の sol 対応（solc の `!abi_compat`、`sol/foundation/fnd.sol`）も実装済み。§9.5 の WAX ローダ（PROTECTED WAX v2 と WAX/WAL v3）はカーネル内に実装済みで、S モードの `fnd_proc_spawn_wax` から使える（`proc_spawn` システムコールはハンドルの実装を待つ）。それ以外（カーネル本体、IPC、システムコール）は未実装。本書は Foundation OS のカーネル ABI を定めます。対象はトラップ、プロセス、スレッド、メモリ所有権、ハンドル、IPC、システムコール、起動の引き渡し、そして C と sol からの利用規約です。背景と開発順序は [実装案](foundation_os_plan.md) を参照してください。

「〜しなければならない (MUST)」「〜してはならない (MUST NOT)」「〜してよい (MAY)」は規範的な要求を表します。

## 1. 範囲と識別子

| 項目 | 値 |
|---|---|
| OS ABI 名 | `foundation`（WAX v3 の `os_id`） |
| OS ABI 版 | major 1、minor 0 |
| 対象 CPU | Extension I と [Extension P](spec_CPU_priv.md) を持つ SRC32 |
| 実行形式 | WAX v2 の `PROTECTED` 付き。[WAX v3](spec_WAX_v3.md) が実装された後は、`os_id = foundation`、`os_major = 1` の v3 も受け付ける |
| 実装言語 | カーネルは C11（`tools/llvm-src32`）。ユーザープログラムは C11、C++17、sol（§13） |

本書が定めないもの：

- ファイル API（パス、ファイルハンドル、ディレクトリ）とサービスのプロトコルは [API 仕様](spec_Foundation_API.md) で定めます。
- ディスク上の構造、UID権限、VFSとの境界は [FS2仕様](spec_FS2.md) で定めます。UIDを認証するカーネル/セッション連携は同仕様の前提条件であり、本ABIにはまだ含まれません。

本書の多バイト整数は、すべて big-endian の符号なし 32-bit です。`i32` と明記したものだけを 2 の補数の符号付き値とします。構造体はすべて 4-byte 境界に置かなければなりません。

## 2. 実行モデル

- カーネルは S モードで動き、**カーネル内では割り込みを禁止します**（トラップ入口で `IE=0` になるのをそのまま使う）。カーネル内でのプリエンプトとネストした割り込みはありません。
- ユーザープロセスは U モードで動きます。プロセスは保護領域の 1 セットとハンドル表を持ち、1〜16 個のスレッドを持ちます。
- スレッドは状態 `READY`、`RUNNING`、`BLOCKED`、`DEAD` のいずれかを取ります。BLOCKED の理由は `SEND`、`REPLY`、`RECV`、`SLEEP`、`WAIT`、`BLK_IO`、`PEC_READ` のいずれかです。
- **idle スレッド**は S モードのカーネルスレッドで、`IE=1` で `HALT` を繰り返します。他に実行できるスレッドがないときだけ選ばれます。
- CPU は 1 つです。カーネルスタックも 1 本だけで、ユーザー文脈は TCB 内の frame（§3.2）に保存します。ブロックするシステムコールは、frame の `r[1]` に結果を書いてから別スレッドへ切り替えます（continuation 方式）。

`/dev/trng` と `/dev/uart0` の読み出しが未準備なら、カーネル IPC は検証済みの要求、応答先、権限をスレッドに保存して `PEC_READ` で待機させます。100 Hz の scheduler tick が再試行し、取得できたときだけ応答を書いて READY にします。待機は期限なしで、他のスレッドは実行できます。終了したスレッドは再試行対象になりません。`/dev/prng` は待機しません。詳細は [API §9.4](spec_Foundation_API.md#94-pec-乱数uart-devtrngdevprngdevuart0)。

## 3. トラップ

### 3.1 ベクタ

カーネルは初期化時に、次の 2 つを設定しなければなりません。

- IRQ slot 0〜15（`0xFFFF0100 + 4n`）と、例外ベクタ語 `0xFFFF0004`、`0xFFFF0008`、`0xFFFF000C` のすべてを、§3.3 のトラップスタブへ向ける。
- IRQ slot には `JR R30` を置き、S バンクの R30 にスタブのアドレスを保持する。

S モードの R30 は、常にスタブのアドレスでなければなりません。idle スレッドを含め、S モードのコードは R30 を変更してはなりません。

### 3.2 文脈 frame

`cpt32_ctx_frame` は 34 語（136 bytes）です。

| 語 | 内容 |
|---:|---|
| 0 | CAUSE |
| 1–27 | R1–R27 |
| 28–30 | 割り込まれたモードの R28–R30。U からのトラップなら `USP`/`UFP`/`UGP`、S（idle）からなら現在のバンクの値 |
| 31 | R31 |
| 32 | EPC |
| 33 | STATUS（`PIE`、`PU`、`PSM` が意味を持つ。`IE` と `U` は保存時の値で、復元では無視する） |

### 3.3 トラップスタブ（ツールチェーンの契約）

カーネルが `cpt32_ctx_frame *kernel_trap(cpt32_ctx_frame *frame)` を定義すると、`tools/llvm-src32` は次の動作をするスタブと起動コードを生成しなければなりません。生成するのは、§3.1 のベクタ設定、グローバル変数 `cpt32_ctx_frame *cpt32_ctx_current`、`uint32_t cpt32_trap_depth` です。`kernel_trap` は `syscall_dispatch` や `irqc_dispatch` と同時に定義できません（コンパイルエラー）。

1. 入口では、`cpt32_trap_depth != 0` かを確認します。0 でなければ、カーネル内で例外が起きたことを意味します。その場合は `*cpt32_ctx_current` を変更せず、カーネルスタック上で `void kernel_panic(uint32_t cause, uint32_t epc)` を呼びます（戻らない）。
2. それ以外の場合、トラップ時点の全レジスタ、CAUSE、EPC、STATUS を `*cpt32_ctx_current` に §3.2 の配置で保存します。`STATUS.PU = 1` ならユーザーバンクを `MFSR USP/UFP/UGP` で読みます。
3. `cpt32_trap_depth = 1` とし、R28 をカーネルスタック頂上（カーネルイメージの `--stack-top`）にして、`kernel_trap(cpt32_ctx_current)` を呼びます。
4. 戻り値の frame `f` を `cpt32_ctx_current` に代入し、`cpt32_trap_depth = 0` とします。
5. `f` の内容を復元します。
   - `EPC = f.EPC`
   - `STATUS = (f.STATUS & (PIE|PU|PSM))`（復元中の IE は 0 のまま）
   - `f.PU = 1` なら `USP/UFP/UGP` を書き、S バンクの R28 をカーネルスタック頂上に戻す。そうでなければ R28 と R29 を直接書く。R30 はスタブのアドレスのまま保つ（S frame の語 30 は記録用で、復元しない）
   - R1–R27 と R31 を書く
6. 最後に `IRET` します。`IRET` の直後、全レジスタは `f` の値と一致しなければなりません（S frame の R30 を除く）。

`cpt32_trap_depth` の初期値は 1 です。そのため、最初の `cpt32_ctx_resume(frame)` より前のトラップは `kernel_panic` に入ります。`cpt32_ctx_resume` は、`cpt32_ctx_current = frame`、depth = 0 にして手順 5 から実行する intrinsic で、戻りません。実装は `tools/llvm-src32`（`backend.py` の `emit_ctx_stub`）、例は `examples/kernel-trap-c` です。

スタブは保護領域に触れません。返す frame が別プロセスのものなら、`kernel_trap` が戻る前に、カーネルが §5.3 の手順で保護領域を書き換えなければなりません。

### 3.4 トラップの分類

| CAUSE | 処理 |
|---|---|
| `0x0000000n`（IRQ） | §10 |
| `0x8003xxxx`（SYSCALL） | §7。EPC はすでに次の命令を指している |
| その他の例外で `STATUS.PU = 1` | そのプロセスを fault で終了する（§9.4）。終了コードは CAUSE の値を `i32` として扱ったもの（負の値になる） |
| その他の例外で `STATUS.PU = 0` | idle スレッドで起きた例外。`kernel_panic` 扱い |

`kernel_panic` は全 IRQ を無効化し、次の順に診断を出力して `HALT` で停止しなければなりません。

1. PCG の文字画面（80×30、SCREEN_MODE 1、スクロール 0）へ、カーネルが VRAM の文字面へ直接書きます。コンソールサービスや IPC には頼りません。ビットマップの貸し出し中でも見えるよう、表示レジスタ（ENABLE、DISPLAY_MODE、FORMAT、OVERLAY、PCG）を戻してから全面を消します。内容は、CAUSE とその種別名、EPC、tick、実行中のプロセス（pid と名前。idle なら `idle or none`）、スタブが最後に保存した文脈（EPC、STATUS、R1〜R31）です。カーネル内の例外では、最後に保存した文脈はカーネルではなく中断されたスレッドのものです。
2. UART へ CAUSE と EPC を出力します。UART のない環境（`src32_testbench`）ではここでバスエラーになりますが、`kernel_panic` に再入したら何も出力せず、そのまま停止します。

停止時の R1 は `0x200 | CAUSE の例外コード（bit 23..16）` です。

## 4. エラーコード

結果は `i32` です。0 以上は成功で、値の意味はシステムコールごとに決まります。負の値はエラーです。共有の errno はありません。

| 値 | 名前 | 値 | 名前 |
|---:|---|---:|---|
| 0 | `FND_OK` | -11 | `FND_E_EXISTS` |
| -1 | `FND_E_BAD_CALL` | -12 | `FND_E_FORMAT` |
| -2 | `FND_E_BAD_ARG` | -13 | `FND_E_UNSUPPORTED` |
| -3 | `FND_E_BAD_HANDLE` | -14 | `FND_E_ADDRESS_CONFLICT` |
| -4 | `FND_E_ACCESS` | -15 | `FND_E_REGION_LIMIT` |
| -5 | `FND_E_NO_MEMORY` | -16 | `FND_E_IO` |
| -6 | `FND_E_LIMIT` | -17 | `FND_E_NO_MEDIA` |
| -7 | `FND_E_PEER_GONE` | -18 | `FND_E_READ_ONLY` |
| -8 | `FND_E_TIMEOUT` | -19 | `FND_E_BUSY` |
| -9 | `FND_E_WOULD_BLOCK` | -20 | `FND_E_KILLED` |
| -10 | `FND_E_NOT_FOUND` | -21 | `FND_E_DEPENDENCY` |

- -1〜-63 はカーネル用です（-22 以降は予約）。
- -64〜-255 はサービスと API 用で、`spec_Foundation_API.md` が定めます。
- -256 以下は予約です。ただし fault による終了コード（CAUSE 値）は例外です。

`FND_E_ACCESS` を返すのは、ポインタ検証の失敗、ケーパビリティの不足、権限の不足の場合です。

## 5. メモリと保護

メモリ管理には 2 つのバックエンドがあります。カーネルは最初のプロセスを起動するときに `CPUID` の `CPU_FEATURES` を読み、Extension V（bit 9、[spec_CPU_vm.md](spec_CPU_vm.md)）があれば**仮想記憶**（§5.5）、なければ **nommu**（MMU のない Linux と同じ考え方。§5.2〜§5.4）を使います。システムコール・IPC・ハンドルの ABI（§6〜§10）はどちらでも同じです。以降、§5.2〜§5.4 は nommu の規則で、仮想記憶で変わる点は §5.5 にまとめます。

### 5.1 物理配置（Main RAM 16 MiB の例）

カーネルは実装されている Main RAM の容量を BMC の `MAIN_RAM_SIZE`（[BMC §1.5](spec_BMC_DMAC.md)）から読みます。0（容量不明）なら 16 MiB とみなします。カーネルより下の配置は容量によらず次の表で固定で、`0x00400000` からこの RAM の末尾まで（最大 256 MiB）がユーザーメモリです。

| 範囲 | 用途 |
|---|---|
| `0x00000000–0x0000FFFF` | stage1 と予約。ユーザーには付与しない |
| `0x00010000–0x000EFFFF` | カーネルイメージ |
| `0x000F0000–0x000FFFFF` | カーネルスタック（頂上 `0x00100000`） |
| `0x00100000–0x003FFFFF` | カーネルヒープ。起動中は `0x00200000` 以降を FNDBOOT の staging に使う（§11） |
| `0x00400000–` | ユーザーメモリ（§5.2） |

### 5.2 ユーザーメモリ（nommu）

アドレス変換がないので、ユーザーのアドレスは物理アドレスそのものです。カーネルはユーザーメモリを 4 KiB のページに分けて管理し、プロセスごとに**連続したページの範囲（アリーナ）**を 1 つ与えます。アリーナにはそのプロセスの全ブロック、読み込む全 WAL、スタック、ヒープが入ります。固定のスロットはありません。

- **再配置可能な実行形式**（[WAX v3 §3.1](spec_WAX_v3.md)）は、空いている連続ページのうち最も低い場所に置きます。必要な大きさは、イメージ・WAL・スタックの予約範囲と、ヒープの予約（実行形式の `heap_hint`、なければ 128 KiB）の合計を 4 KiB に切り上げたものです。カーネルは読み込むときに、実行形式の持つアドレスへ置き場所との差を足します（Linux の nommu の FLAT 形式と同じです）。同じ画像を何個でも同時に起動できます。置ける連続範囲がなければ `FND_E_NO_MEMORY` です。
- **固定アドレスの実行形式**（v2 と、再配置可能でない v3）は、`base_addr` からスタックの上端までの予約範囲をそのまま 4 KiB 境界に広げた範囲を取ります。その範囲のページが 1 つでも使用中なら `FND_E_ADDRESS_CONFLICT`、実在する RAM やユーザーメモリの外にかかるなら `FND_E_NO_MEMORY` です。WAL は、その範囲の中に置きます（§9.5）。
- **ヒープ**：最も高いブロックの末尾から始まり、アリーナの末尾か、その上にある次の予約（スタックなど）までです。ヒープがアリーナの最後にあるとき、`mem_grow` は直上の空きページを同じアリーナに加えて伸ばします（Linux の nommu の `brk` と同じで、**動かすことはできません**）。直上を別のプロセスが使っていれば `FND_E_NO_MEMORY` です。大きなヒープが要るプログラムは、`heap_hint` で最初から予約してください。
- **解放**：プロセスが終了すると、書き込んだ範囲（保護領域のうちカーネルが所有するもの）をゼロにしてから、アリーナ全体をプールへ返します。空きページは常にゼロです。
- コードをカーネルのイメージにリンクして動かす `fnd_proc_create_builtin` は、旧スロットの範囲（`fnd_slot_base`／`fnd_slot_end`）の指定を受け付け、その範囲の上端 `ram_size` byte を同じ方法で取ります。ほかの用途ではスロットを使いません。
- プロセスの数は 7 個のままです（メモリの区画とは無関係な、プロセス記録の上限）。

### 5.3 保護領域の割り当てと切り替え

プロセスの 8 領域は、次の順に割り当てます。合計が 8 を超える場合、spawn は `FND_E_REGION_LIMIT` で失敗します。

1. region 0：スタック（RW）
2. 次の region から：WAX のブロック。隣接していて権限が同じブロックだけをまとめる（[WAX v3 §5](spec_WAX_v3.md) と同じ規則）
3. ヒープ（RW）：最も高いアドレスのブロック末尾を 8 byte 境界に切り上げた位置から始まる（`mem_grow` の delta と同じ単位。間の byte は 0）
   - そのブロックが RW なら、同じ region を伸ばす
   - そうでなければ region を 1 つ予約する。予約できない場合、`mem_grow` は `FND_E_REGION_LIMIT` を返す
4. MMIO の付与（§10.3）

別プロセスのスレッドへ切り替えるときは、次の順で領域を書き換えなければなりません。同じプロセス内のスレッド切り替えでは、領域を書き換えません。

1. 8 領域すべての FLAGS を 0 にする。
2. 新しいプロセスの各領域について START と END を書く。
3. 新しいプロセスの各領域について FLAGS を書く。

### 5.4 ユーザーポインタの検証

（仮想記憶では 3. を「そのプロセスのページ表で、必要な権限を持つユーザーページに載っている」と読み替えます。§5.5）

カーネルがユーザーメモリを読み書きする前には、必ず次の 3 つを検証しなければなりません。範囲は `[addr, addr+len)`（`len = 0` なら addr は検査しない）です。

1. 加算が 32-bit でオーバーフローしない。
2. `0xFFFF0000` 以上に触れない。
3. 範囲のすべての byte が、そのプロセスの RAM 領域（MMIO の付与は含まない）のうち、必要な権限（読むなら R、書くなら W）を持つ領域に含まれる。

検証に失敗した場合、メモリを一切変更せずに `FND_E_ACCESS` を返します。構造体へのポインタが 4-byte 境界にない場合は `FND_E_BAD_ARG` を返します。

- メモリの解放と再利用：前のプロセスに付与した範囲は、次のプロセスに付与する前にゼロにしなければなりません。
- ヒープの伸長は `mem_grow` で行います。スタックの下端を越えることはできません。

#### 5.4.1 カーネル内の同期DMAコピー

検証後の物理メモリコピーは、重ならないMain RAM範囲の256 bytes以上についてDMAC channel 1を使えます。channel 0はdisk I/O用、channel 1はカーネルコピー用に予約します。VMでは送信元・宛先双方のページ境界で分割し、物理アドレスに変換してから転送します。権限検証を省略したり、ユーザーの仮想アドレスをDMACへ直接渡したりしません。

転送は割り込み禁止のカーネル内で完了までポーリングする同期処理です。DMAC ch1のIRQは使用せず、BMC優先度はCPUと同じP0とします。整列位置が一致する場合は先頭と末尾をCPUで処理し、本体を32-bit DMAで転送します。位置が異なる場合は8-bit DMAを使います。他チャネルのenable/IRQ/statusは保持します。

小さい範囲、重なった範囲、Main RAM外、DMAC未搭載・無効、BMC調停無効、ch1使用中ではCPUコピーを使います。DMAがエラーで停止した場合も、重ならない元の範囲全体をCPUで再コピーします。DMAを使う場合もAPIの完了時点で全データが転送済みであり、プロセスの待機理由やIPC ABIは変わりません。`FND_COPY_DMA_MIN=0` の比較用ビルドではDMA経路を無効にできます。

## 6. ハンドル

| 種類 | 作成 | できること |
|---|---|---|
| `PORT` | `port_create` | `recv`、`chan_create`、`svc_register`、`irq_bind` |
| `CHANNEL` | `chan_create`、`svc_lookup`、IPC での転送 | `call` |
| `PROCESS` | `proc_spawn` | `wait`、`proc_kill` |
| `THREAD` | `thread_create` | `wait` |

- ハンドルの値は 1〜63 で、プロセスごとの番号です。0 は「ハンドルなし」を表します。新しいハンドルには、空いている最小の番号を割り当てます。上限を超えた場合は `FND_E_LIMIT` です。
- `handle_dup` は、同じオブジェクトを指す別のハンドルを作ります。
- オブジェクトは、それを指すハンドルが 0 個になったときに破棄されます。
- **PORT の破棄**：
  - 待機中の `call` を `FND_E_PEER_GONE` で終わらせる。
  - その PORT を指す全 CHANNEL を無効にする（以後の `call` は `FND_E_PEER_GONE`）。
  - サービス登録と IRQ の割り当てを解除する。
- **CHANNEL の破棄**：その CHANNEL の最後のハンドルが閉じられると、相手の PORT に `CLIENT_GONE` 通知（§8.5）を 1 回だけ届けます。
- カーネルドライバの CHANNEL（API仕様 §9.1 の `dev.fb0`）は PORT を持ちません。`ipc_call` は通常のメモリ検証後、カーネル内で同期実行します。最後のcloseは `CLIENT_GONE` の代わりにドライバの解放処理を呼びます。複製・転送・起動時の継承は同じ参照を共有し、プロセス終了時も通常のハンドル破棄で解放します。`svc_lookup("dev.fb0")` はopen用CHANNELを返し、この名前の `svc_register` は EXISTS です。

## 7. システムコール規約

- `SYSCALL n`：`n` はシステムコール番号（§9）です。context byte は 0 でなければならず、0 以外なら `FND_E_BAD_CALL` を返します（将来の版の識別用に予約）。
- 引数は R1〜R4、結果は R1（`i32`）です。
- **R1 以外のレジスタはすべて保存されます**。引数レジスタ R2〜R4 も、U バンクの R28〜R30 も保存されます。この規則により、C の `cpt32_syscall` も sol の `!asm` ラッパー（§13）も、レジスタの退避をせずに呼び出せます。
- 4 個を超える引数は、構造体へのポインタで渡します。
- 未定義の番号は `FND_E_BAD_CALL` を返します。
- ブロックするシステムコールは、起床した時点で結果が R1 に入った状態で戻ります。

### 7.1 タイムアウト

タイムアウトの単位は tick（10 ms）です。実時間からの換算には `time_get(2)` または `sys_info(0)` の `tick_hz`（100）を使い、1 tick = 1 ms と仮定してはなりません。0 はブロックせず、条件が満たされていなければ `FND_E_WOULD_BLOCK` を返します。`0xFFFFFFFF`（`FND_FOREVER`）は無期限です。それ以外の値は、起点から数えてその tick 数の後に `FND_E_TIMEOUT` を返します。

## 8. IPC

### 8.1 構造体

`fnd_msg`（32 bytes）：

| Off | 名前 | 意味 |
|---:|---|---|
| 0 | `op` | 操作コード。応答では `fnd_status` を入れる慣例とする。`0xFFFF0000` 以上はカーネル通知用に予約 |
| 4 | `flags` | bit 0 `XFER_HANDLE`：`a3` で渡すハンドルを転送する。その他の bit は 0 |
| 8–20 | `a0`–`a3` | 引数 |
| 24 | `buf` | データバッファのアドレス |
| 28 | `len` | データ長（in）／コピーされた長さ（out） |

`fnd_recv`（64 bytes）：

| Off | 名前 | 方向 | 意味 |
|---:|---|---|---|
| 0 | `msg` | out | 受け取った `fnd_msg`。`buf` は受信側の `data` に書き換え、`len` は実際にコピーした byte 数 |
| 32 | `data` | in | 受信用バッファ |
| 36 | `cap` | in | 受信用バッファの容量 |
| 40 | `token` | out | 応答用の token（1 以上）。通知なら 0 |
| 44 | `badge` | out | 送信元の CHANNEL の badge |
| 48 | `reply_cap` | out | クライアントが用意した応答データの容量 |
| 52 | `total` | out | 送信側の `len`。`total > msg.len` なら切り詰められている |
| 56 | `timeout` | in | §7.1 |
| 60 | — | — | 予約。0 にする |

### 8.2 データ転送

- 1 回の転送で送れるデータは最大 4096 bytes（`FND_IPC_MAX_DATA`）です。超える場合は `FND_E_LIMIT` を返します。
- カーネルは送信元と受信先のバッファを §5.4 の規則で検証し、プロセス間で直接コピーします。共有メモリはありません。
- 要求データが受信側の `cap` より大きい場合は、`cap` まで切り詰めて届けます。切り詰めたかどうかは `total` で分かります。
- 応答データがクライアントの容量より大きい場合は、届けずに `FND_E_LIMIT` を返します。このとき token は有効なままです。

### 8.3 呼び出し

| 操作 | 動作 |
|---|---|
| `ipc_call(ch, req, rep)` | `req` とそのデータを送り、応答を待つ（`SEND` → `REPLY`）。<br>`rep.buf`/`rep.len` には、受け取るデータの置き場と容量を入れておく。<br>戻ると `rep` に応答の `fnd_msg` が入り、`rep.buf` は変わらず、`rep.len` はコピーした byte 数になる。<br>R1 は配送の結果で、サーバー側の処理結果は `rep.op` に入る |
| `ipc_recv(port, rcv)` | 通知か要求を 1 つ受け取る |
| `ipc_reply(token, msg)` | 応答を返す。クライアントが終了していれば `FND_E_PEER_GONE`（token は破棄される） |
| `ipc_reply_recv(token, msg, port, rcv)` | 応答と受信を続けて行う。<br>応答が `FND_E_PEER_GONE` なら無視して受信へ進む。<br>応答がそれ以外のエラーなら、受信せずにそのエラーを返す |

- `call` にタイムアウトはありません。
- 受け取る順序は、IRQ 通知、`CLIENT_GONE` 通知、待機中の要求（FIFO）の順です。
- 優先度の継承はありません。

### 8.4 ハンドルの転送

`flags.XFER_HANDLE` を立てた要求や応答では、送信側の `a3` に入っているハンドルが受信側へ移ります。

- 受信側では `a3` が受信側の新しいハンドル番号に書き換わります。
- 送信側のハンドルは閉じられます。
- 転送できない場合（ハンドルが無効、受信側の表が満杯）は、その送信全体が失敗します。

### 8.5 通知

| op | 名前 | 内容 |
|---|---|---|
| `0xFFFF0001` | `FND_NOTIFY_IRQ` | `a0` = IRQ 番号。IRQ ごとに 1 つの pending にまとめる |
| `0xFFFF0002` | `FND_NOTIFY_CLIENT_GONE` | `badge` = 破棄された CHANNEL の badge |

- 通知は `token = 0` で、応答してはなりません。
- 通知は、PORT に紐づいたオブジェクト自体を pending として保持します。そのため通知の数は無制限には増えません。

### 8.6 badge とサービス登録

- `chan_create(port, badge)` の badge は 1〜`0x7FFFFFFF` でなければなりません。
- `svc_lookup` が作る CHANNEL には、カーネルが一意の badge（bit 31 = 1）を割り当てます。これにより、サーバーはセッションごとにクライアントを区別できます。
- サービス名は `[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*` の形で、31 bytes 以下です。
- 登録の上限は 32 件です。同じ名前の二重登録は `FND_E_EXISTS` になります。
- 登録は、その PORT が破棄されると消えます。

## 9. システムコール一覧

**Cap** はそのシステムコールに必要なケーパビリティです（§9.1）。

| 番号 | 名前 | 引数 (R1–R4) | 結果 | Cap |
|---:|---|---|---|---|
| 0x00 | `sys_info` | sel, buf, cap | 必要な byte 数（`cap` まではコピー済み） | — |
| 0x01 | `ipc_call` | ch, req, rep | 0 | — |
| 0x02 | `ipc_recv` | port, rcv | 0 | — |
| 0x03 | `ipc_reply` | token, msg | 0 | — |
| 0x04 | `ipc_reply_recv` | token, msg, port, rcv | 0 | — |
| 0x05 | `port_create` | — | ハンドル | — |
| 0x06 | `chan_create` | port, badge | ハンドル | — |
| 0x07 | `handle_close` | h | 0 | — |
| 0x08 | `handle_dup` | h | ハンドル | — |
| 0x09 | `thread_create` | entry, sp, arg | ハンドル | — |
| 0x0A | `thread_exit` | code | 戻らない | — |
| 0x0B | `yield` | — | 0 | — |
| 0x0C | `sleep` | ticks | 0 | — |
| 0x0D | `wait` | h, timeout, code_ptr | 0 | — |
| 0x10 | `proc_spawn` | spawn | PROCESS ハンドル | （§9.2） |
| 0x11 | `proc_exit` | code | 戻らない | — |
| 0x12 | `proc_kill` | h | 0 | — |
| 0x13 | `proc_startup` | buf, cap | 起動ブロックの byte 数 | — |
| 0x14 | `proc_fork` | — | 親: PROCESS ハンドル、子: 0 | （§9.6） |
| 0x18 | `mem_grow` | delta | 伸長前のヒープ末尾 | — |
| 0x20 | `irq_bind` | irq, port | 0 | `IRQ` |
| 0x21 | `irq_ack` | irq | 0 | `IRQ` |
| 0x28 | `blk_io` | blk | 0 | `DISK` |
| 0x30 | `time_get` | sel | 値 | — |
| 0x31 | `svc_register` | name, port | 0 | `SVC` |
| 0x32 | `svc_lookup` | name | CHANNEL ハンドル | — |
| 0x3F | `debug_write` | buf, len | len | — |

各システムコールの補足：

- `thread_create`：新しいスレッドは、PC = entry、USP = sp、R1 = arg で始まります。その他のレジスタは 0 で、UGP は親プロセスと同じです。entry は X 権限を持つ範囲の 4-byte 境界、sp は RW 権限を持つ範囲の 8-byte 境界でなければなりません。
- `wait`：対象のスレッドかプロセスの終了を待ち、`code_ptr`（0 なら書かない）に `i32` の終了コードを書きます。
- `thread_exit`：プロセスの最後のスレッドが `thread_exit` すると、その終了コードでプロセスも終了します。
- `sleep(0)`：`yield` と同じです。
- `mem_grow`：`delta` は 8 の倍数でなければなりません。
- `time_get` の sel：
  - 0 = tick の下位 32 bit
  - 1 = tick の上位 32 bit
  - 2 = 1 秒あたりの tick 数
  - 3 = RTC 秒の下位 32 bit
  - 4 = RTC 秒の上位 32 bit
- `debug_write`：カーネルのデバッグコンソール（UART）へ出力します。1 回あたり 256 bytes まで書け、それを超える分は切り詰めます。
- `sys_info` の sel：
  - 0 = `fnd_sysinfo`（32 bytes）。順に `abi_major`、`abi_minor`、`tick_hz`、`ram_bytes`、`free_bytes`、`proc_count`、`boot_drive`、予約。`ram_bytes` は §5.1 の Main RAM 容量、`free_bytes` は nommu ではアリーナに使われていないページの合計（仮想記憶では未使用のフレームの合計）
  - 1 = プロセス一覧。1 件 48 bytes で、順に `pid`、`parent`、`state`（0 = 実行可能、1 = BLOCKED、2 = 終了済み）、`priority`、`exit_code`、`mem_bytes`、`threads`、`fault_epc`、`name[16]`
  - 2 = CPU 統計。16 bytes のヘッダ（`elapsed_cycles`、`kernel_cycles`、`idle_cycles`、`clock_hz`）に、プロセスごとの 8 bytes（`pid`、`user_cycles`）が続きます。すべて u32、カウンタは 2^32 で wrap します。レコード順は sel 1 と同じで、終了記録も含み、再利用時は新 PID のカウンタを 0 にします。PeC timer ch1 を IRQ なしの 48 MHz free-running clock としてカーネルが予約します。スケジューラの起動時から計測し、HALT 中の経過も idle に含みます。使用率はスナップショット間の符号なし差分で計算し、間隔は約89秒未満とします。
    - `kernel_cycles` は C の trap ハンドラ入口から scheduler 終了までの概算です。アセンブリの context save/restore と C 計測関数の境界外の命令は、前後の user/idle に含まれます。厳密な OS オーバーヘッドとは一致しません。`user_cycles` はプロセスの全スレッドの合計、syscall 処理時間は kernel 側です。ch1 による計測自体の実行コストもあります。
  - 3 = `fnd_meminfo`（16 bytes）。順に `ram_bytes`、`vram_bytes`、`pcmram_bytes`、予約（0）。ハードウェアの実装容量で、BMC のメモリ容量レジスタ（[BMC §1.5](spec_BMC_DMAC.md)）の値です。`ram_bytes` は sel 0 と同じ（容量不明なら 16 MiB）。`vram_bytes` と `pcmram_bytes` は BMC の値そのままで、0 は容量不明を表します
  - その他は `FND_E_BAD_ARG`

### 9.1 ケーパビリティ

プロセスごとに bit mask で持ちます。

| bit | 名前 | 意味 |
|---:|---|---|
| 0 | `PRIO` | 自分より高い優先度でプロセスを起動できる |
| 1 | `IRQ` | `irq_bind`/`irq_ack` |
| 2 | `DISK` | `blk_io` |
| 3 | `SVC` | `svc_register` |
| 4 | `MMIO` | MMIO を付与したプロセスを起動できる |

起動するプロセスに与えられるのは、起動する側が持つケーパビリティの部分集合だけです。

### 9.2 `proc_spawn` と `fnd_spawn`（48 bytes）

| Off | 名前 | 意味 |
|---:|---|---|
| 0 | `image` / 4 `image_len` | メモリ上の WAX。カーネルが最初に staging へ複製してから検証する |
| 8 | `startup` / 12 `startup_len` | 起動ブロック（§9.3）。4096 bytes 以下 |
| 16 | `handles` / 20 `handle_count` | 子へ渡すハンドルの配列（16 個以下）。各ハンドルは dup され、子では 1 + 配列の添字の番号になる |
| 24 | `priority` | 0 = DRIVER、1 = SERVICE、2 = APP。起動する側より小さい値（より高い優先度）は `PRIO` が必要 |
| 28 | `caps` | §9.1 |
| 32 | `grants` / 36 `grant_count` | MMIO の付与（§10.3）。1 件 12 bytes で `start`、`end`、`perm`。`MMIO` が必要 |
| 40 | `name` | 15 bytes 以下の名前（`/os/proc` 用） |
| 44 | `libs` | WAL の一覧（§9.5）へのポインタ。0 なら一覧なし |

`image_len` の上限は `FND_SPAWN_IMAGE_MAX`（3 MiB）。WAX と全 WAL の合計（各イメージ後の 4-byte 境界へのパディングを含む）は staging 領域 `0x00100000..0x00400000` に収めます。上限超過は、イメージをコピーする前に `FND_E_LIMIT` で拒否します。

`libs` が指す一覧（`fnd_spawn_libs`、8 + 8 × `count` bytes）：

| Off | 名前 | 意味 |
|---:|---|---|
| 0 | `count` | 件数。16 以下 |
| 4 | — | 予約。0 にする |
| 8 + 8i | `image` / 12 + 8i `image_len` | i 番目の WAL のメモリ上のイメージ |

- カーネルは `image` と同じく、一覧と各 WAL を staging へ複製してから検証します。複製した後に呼び出し側がメモリを変えても、起動には影響しません。
- `count` が 16 を超える、または予約欄が 0 でなければ `FND_E_BAD_ARG` です。v2 の WAX と依存のない v3 には、`libs` は使われません（あっても無視します）。

- 起動は原子的に行わなければなりません（[WAX v3 §7](spec_WAX_v3.md) と同じ）。どこかで失敗した場合、何も確保されていない状態に戻します。
- PROTECTED のない WAX は `FND_E_FORMAT` で拒否します。S モードで実行してはなりません。
- 入口の状態は次のとおりです。
  - Normal モード、`PC = entry`、`USP = user_stack_top`、`UFP = 0`、`UGP = base_addr`
  - R1〜R27 と R31 は 0
  - `IE = 1`
  - スタックはゼロクリア済み

### 9.3 起動ブロック

カーネルは起動ブロックを解釈せず、そのまま複製します。子は `proc_startup` で、自分のバッファへコピーさせて読みます。ブロックの形式は libfnd と sol ライブラリが従う OS ABI の慣例として、次のように定めます。

| Off | 名前 | 意味 |
|---:|---|---|
| 0 | magic | `0x464E4453`（`FNDS`） |
| 4 | version | 1 |
| 8 | size | ブロック全体の byte 数 |
| 12 | argc | 引数の数 |
| 16 | argv_off | argc 個の u32 オフセットの配列（ブロック先頭から） |
| 20 | cwd_off | カレントディレクトリ（絶対パス）の位置 |
| 24 | role_count | 役割表の件数 |
| 28 | roles_off | `(role, handle)` の組の配列 |

- 文字列は NUL で終わります。
- role の値：1 `STDIN`、2 `STDOUT`、3 `STDERR`、4 `FS`（fs への CHANNEL）、5 `CONSOLE`。
- `handle` は子側でのハンドル番号です（§9.2 の規則で決まる）。

### 5.5 仮想記憶（Extension V）

CPU が Extension V を報告するとき（`CPU_FEATURES` bit 9）、カーネルは `VCTL.EN` を 1 にして、U モードの命令フェッチ・ロード・ストアをページ表で変換します。S モードは変換しません（カーネルは物理アドレスのまま）。設計の理由は [foundation_vm_plan.md](foundation_vm_plan.md) にあります。

- **アドレス空間**：プロセスごとに 2 段のページ表（4 KiB ページ）を持ちます。プロセスを切り替えるとき、`PTBR` を書きます（TLB は全消去されます。ASID はありません）。保護領域（§5.3）は使いません。同じプロセスの中のスレッド切り替えでは何もしません。
- **物理フレーム**：`0x00400000` 以降、Main RAM の末尾までを 4 KiB のフレームとして管理します（最大 256 MiB）。先頭の 64 KiB は `blk_io` の転送用バッファに予約します。フレームは付与する直前にゼロにし、参照数で所有を管理し、プロセスの終了時にページ表を返し、データフレームは最後の参照がなくなった時点で返します。アリーナ（§5.2）は使いません。
- **配置**：イメージは `0x00001000` 以上 `0x10000000` 未満のどこにでも置けます（この範囲を超える・ページ 0 を含む場合は `FND_E_FORMAT`）。WAL もこの範囲に first fit で置きます。リンクアドレスが同じイメージを複数同時に起動できるので、`FND_E_ADDRESS_CONFLICT` は返りません。再配置可能な実行形式（[WAX v3 §3.1](spec_WAX_v3.md)）は、仮想記憶ではリンクしたアドレスのまま読み込みます（アドレス空間がプロセスごとなので、動かす必要がありません）。固定アドレスの既存のイメージもそのまま動きます。再配置可能な実行形式（[WAX v3 §3.1](spec_WAX_v3.md)）は、仮想記憶ではリンクしたアドレスのまま読み込みます（アドレス空間がプロセスごとなので、動かす必要がありません）。固定アドレスの既存のイメージもそのまま動きます。
- **権限はページ単位**：1 ページには 1 組の権限しかありません。ブロックの境界が同じページにある場合、そのページは両方の権限の和になります（例：RX のコードと RW のデータが同じページなら RWX）。Foundation のイメージ（サービス・シェル・コマンド・libfnd の WAL）は、llvm-src32 の `--page-align` でコード（RX）の次の RW データを 4 KiB 境界から始め、WAX v3 ではそのブロックの整列も 4096 にします。このためコードのページは書けず、データのページは実行できません（W^X）。`--page-align` を付けないイメージは、ページを共有する分だけ権限が広がります。`.rodata` は現在 RW データの中にあり、R 専用ブロックには分けていません。
- **領域の上限**：保護領域がないので、MMIO の付与や `mem_grow` が `FND_E_REGION_LIMIT` になることはありません。ただしイメージ自体は、ローダーの規則どおり 8 個の領域（隣接して権限が同じブロックをまとめたもの）までです。
- **ヒープ**：最も高いブロック末尾の次のページ境界から始まり、次の予約（スタック・WAL・MMIO の付与）の手前のページ境界、または `0x80000000` の手前まで伸ばせます。`mem_grow` は新しいページを足すだけで、物理的に連続している必要はありません。
- **MMIO の付与**（§10.3）：付与された範囲を含むページを、Device 属性・同じ仮想アドレスでマップします。窓の検査は byte 単位のままですが、ハードウェアはページ単位でしか制限できません。たとえば PeC の UART（`0x80040000–0x8004002F`）を付与されたプロセスは、同じページにある PeC のほかのレジスタにも触れます。付与は信頼できるサービスにだけ許してください。カーネルは vm のとき、付与のページ範囲が届く先がすべて付与可能な窓の中にあること、排他窓（VRAM・PCMRAM）なら空いていることも検査します（`FND_E_ACCESS`／`FND_E_BUSY`）。
- **ユーザーポインタ**（§5.4）：カーネルは S モード（変換なし）で動くので、ユーザーの仮想アドレスをページ表から物理アドレスへ自分で変換し、ページごとに複写します。Device ページと、`U`・`R`/`W`/`X` が足りないページは `FND_E_ACCESS` です。
- **`blk_io`**（§10.4）：DMA は変換されないので、転送はカーネルのバッファを経由し、完了後に複写します。
- **フォールトと COW**：`proc_fork`（§9.6）で共有した書き込み可能ページは、PTE の W を落として SW bit 9（COW）を立てます（SW bit 8 は所有フレーム）。書き込み保護フォールト `0x8006000A` が COW ページを指す場合、共有中なら新しいフレームへ 4 KiB をコピーし、最後の所有者ならコピーせず W を復元します。TLB を消去して同じ EPC の命令を再実行します。真の読み取り専用ページや未マップページへのアクセス、COW 解決のメモリ不足を含む、ほかの U モードフォールトはプロセスを終了します。終了コードは CAUSE です。需要ページングはないので、ページは `proc_spawn` と `mem_grow` が先にすべてマップします。
- **`sys_info`**：sel 0 の `free_bytes` は未使用の物理フレームの合計、sel 1 のプロセスごとのメモリはマップされた所有フレーム（ページ表を除く）の合計です。共有フレームは各プロセスの値に含まれるため、プロセスの値の合計は物理使用量と一致しません。
- **限界**：プロセスは 7 個、スレッドは 8 個までのままです（§9）。

### 9.4 終了と後始末

プロセスが終了する原因は、`proc_exit`、最後のスレッドの `thread_exit`、fault、`proc_kill` のいずれかです。`proc_kill` による終了コードは `FND_E_KILLED` です。カーネルは次の順で後始末をしなければなりません。

1. 全スレッドを `DEAD` にする。
2. 進行中の `blk_io` があれば IDC に ABORT を発行し、完了を待つ。DMA が完了するまで、転送先のメモリを解放してはならない。
3. 全ハンドルを閉じる。これにより `PEER_GONE` と `CLIENT_GONE` が届く。
4. IRQ の割り当てとサービス登録を解除する。
5. 付与した範囲をゼロにし、アリーナを返す（仮想記憶ではページ表とフレームを返す。§5.5）。
6. `wait` している側を起こす。

終了したプロセスの記録は、`sys_info` で読めるように、PROCESS ハンドルが閉じられるまで残します。

### 9.5 WAX の読み込み

`proc_spawn` と §11 の同梱モジュールの起動は、どちらも次の規則で WAX を読み込みます。[WAX v3 §4.1・§7](spec_WAX_v3.md) が OS に委ねている選択と配置の方針も、ここで定めます。

- **受け付ける形式**：PROTECTED の WAX v2 と、type が WAX の v3。v3 では、すべての画像（WAX と WAL）が `call_abi = 1`、`os_id = foundation`、`os_major = 1`、`os_min_minor` ≤ カーネルの minor（現在 0）でなければなりません。検証と束縛は WAX v3 §2〜§7 に従います。
- **置き場所**（nommu は §5.2、仮想記憶は §5.5）：WAX の予約範囲（最も低いブロックから最も高いブロックの末尾まで、穴を含む）、スタック、読み込む全 WAL は、1 つの範囲（固定アドレスの実行形式では予約範囲を 4 KiB 境界に広げたもの、再配置可能な実行形式ではアリーナ）に収まらなければなりません。収まらなければ `FND_E_FORMAT` です。
- **提供者の選択**：候補は、カーネルに渡されたライブラリ一覧のうち、検証に通る WAL で `image_id` と `abi_major` が要求と一致するものです。その中で `abi_minor` が最大のものを選び、同じなら一覧で先にあるものを選びます。選んだ WAL の `abi_minor` が、同じ ID へのすべての要求の `min_minor` の最大値に満たなければ失敗します。
- **WAL の配置**：依存の帰りがけ順（依存される WAL が先）に、その範囲の先頭からの first fit で置きます。各 WAL の基底 B は、その WAL のブロック整列の最大値に揃えた、WAX の予約範囲・スタック・配置済みの WAL と重ならない最も低いアドレスです。
- **上限**：1 回の起動で画像は 8 個（WAX と WAL 7 個）まで、1 画像あたりブロックは 32 個、依存は 8 件までです。超えると `FND_E_LIMIT` です。
- **エラー**：WAX v3 §7 の論理分類を次のように対応させます。

| 分類 | 結果 |
|---|---|
| FORMAT | `FND_E_FORMAT` |
| UNSUPPORTED、ABI_MISMATCH | `FND_E_UNSUPPORTED` |
| DEPENDENCY_MISSING、DEPENDENCY_CONFLICT、DEPENDENCY_CYCLE、SYMBOL_MISSING、SYMBOL_MISMATCH | `FND_E_DEPENDENCY` |
| REGION_LIMIT | `FND_E_REGION_LIMIT` |
| RESOURCE_LIMIT | `FND_E_LIMIT` |
| 固定アドレスの範囲が使用中（nommu のみ） | `FND_E_ADDRESS_CONFLICT` |
| 連続した空きページがない（nommu） | `FND_E_NO_MEMORY` |

**ライブラリの置き場と渡し方（2026-10-22 決定）**：

- インストール済みの WAL は `/lib/<image_id>.<abi_major>.wal` に置きます（例：libfnd は `/lib/fnd.1.wal`）。同じ ID と major の WAL は 1 つだけで、minor を上げるときはファイルを置き換えます。ファイル名は探す場所を決めるだけで、選んだ WAL の `image_id`、`abi_major`、`abi_minor` が要求を満たすかは、カーネルがイメージの中身で確かめます（WAX v3 §4.1）。
- カーネルは fs を読みません。起動する側が、WAX の依存表から必要な WAL を（WAL の依存もたどって）`/lib` から読み、`proc_spawn` の `libs`（§9.2）で渡します。libfnd では `fnd_proc_exec` がこれを行います（[API 仕様 §11.6](spec_Foundation_API.md)）。
- カーネル内で起動する場合（§11 の同梱モジュールやテスト）は、S モードの `fnd_proc_spawn_wax` に同じ形の一覧を渡します。

### 9.6 `proc_fork`（SYSCALL 0x14）

引数なし。呼び出し元のアドレス空間と呼び出したスレッドのコンテキストを複製し、親には子の PROCESS ハンドル（`wait`・`proc_kill`・`handle_close` 用）、子には 0 を返します。子は同じシステムコールの次の命令から再開します。エラーは親だけに負の `FND_E_*` を返し、子は作りません。

- VM バックエンド専用です。nommu では `FND_E_UNSUPPORTED`。生存スレッドが 1 本でなければ `FND_E_BUSY`。THREAD ハンドル、MMIO 付与（VRAM・PCMRAM を含む）、Device ページ、所有しない書き込み可能マッピングがあれば `FND_E_ACCESS` です。
- 子は新 PID、親 PID、同じ名前・優先度・capabilities・起動ブロック・ヒープ境界を持ちます。生存スレッドは呼び出した 1 本だけで、CPU 統計は 0 から始まります。
- ハンドルは穴を含めて同じ番号に複製し、同じカーネルオブジェクトを参照します。ファイルの位置やセッション状態も共有されます。親に返す子の PROCESS ハンドル自体は子へ渡しません。サービス登録・IRQ バインド・受信済み reply token の所有権は子に移しません。
- 所有する RX ページは読み取り専用のままフレームを共有し、RW ページは COW として共有します。ページ表は子専用に確保します。確保失敗では子の参照とページ表を解放し、親の PTE は変更しません。プロセス・スレッド・親のハンドル枯渇は `FND_E_LIMIT`、ページ表の確保失敗は `FND_E_NO_MEMORY` です。
- カーネルがユーザーメモリへ書く場合も COW が必要です。§5.4 の W 検査は全範囲の権限を確認した後、COW を解決します。確保できなければ範囲検査失敗（既存 syscall の `FND_E_ACCESS`）となり、出力は書きません。既に私有化できたページは私有のままです。IPC・wait・DMA 完了の出力先はブロックする前に私有化されます。単一スレッド制約により、ブロック中の検査済み出力先が fork で再共有されることはありません。

C では `<foundation/sys.h>` の `fnd_sys_proc_fork()`、libfnd の `fnd_proc_fork()`、sol では `sol/foundation/fnd.sol` の `fnd_proc_fork` を使えます。

## 10. スケジューリング、IRQ、デバイス

### 10.1 スケジューラ

- 優先度は 0 DRIVER、1 SERVICE、2 APP、3 IDLE です（値が小さいほど優先）。同じ優先度の中では FIFO のラウンドロビンにします。
- tick は PeC timer ch0 の periodic で、100 Hz（1 tick = 10 ms、RELOAD = 480,000）です。IRQ4 で受けます。
  - timer ch0 はカーネル専用です。
  - RTC alarm は v1 では使いません。
  - IRQ0（VSync）を時刻に使ってはなりません。
- タイムスライス（クオンタム）は 1 tick（10 ms）です。使い切ったスレッドは同じ優先度のキューの末尾へ回します。
- より高い優先度のスレッドが起きた場合、カーネルからユーザーへ戻る時点でそのスレッドに切り替えます。

### 10.2 IRQ

| IRQ | 扱い |
|---:|---|
| 0, 3, 5–12 | `irq_bind` できる |
| 1, 2, 4 | カーネル専用（DMAC、IDC、timer） |
| 13–15 | 予約 |

`irq_bind(irq, port)` と `irq_ack(irq)` は `IRQ` capability が必要です。
範囲外（16 以上）は `FND_E_BAD_ARG`、カーネル専用・予約 IRQ は `FND_E_ACCESS`、
PORT 以外または無効なハンドルへの bind は `FND_E_BAD_HANDLE` です。
IRQ は bind したプロセスが所有し、既に割り当てられた IRQ の bind は、同じ所有者・PORT でも
`FND_E_BUSY` になります。bind は既存の PENDING を消さず ENABLE を 1 にします。
1 つの PORT へ複数の IRQ を割り当てられます。

PORT の最後のハンドルを閉じるか、所有プロセスが終了（kill・例外を含む）すると、
割り当てと未受信の通知を破棄し、ENABLE を 0 にして PENDING を W1C します。
PORT の複製・転送では IRQ 所有者は変わりません。所有プロセス終了時は、他プロセスが
PORT を保持していても解放します。IRQ の割り当て自体は PORT の寿命を延ばしません。

割り当て済みの IRQ は、次の手順で扱います。

1. カーネルは IRQ を受けたら、その source の IRQC ENABLE bit を 0 にし、PENDING は残したまま `FND_NOTIFY_IRQ` を届けます。
2. サービスは原因デバイスを ACK してから、`irq_ack` を呼びます。
3. `irq_ack` で、カーネルは PENDING を W1C し、ENABLE を 1 に戻します。

通知は受信待ちのスレッドを起こすか、次の `recv` まで IRQ ごとに 1 件保持します。
`msg.a0` が IRQ 番号、`token`・`badge`・`reply_cap`・`total` は 0 で、データはありません。
ACK するまでは同じ IRQ の通知を追加しません。`irq_ack` は未割り当てなら
`FND_E_NOT_FOUND`、所有者以外なら `FND_E_ACCESS`、通知をまだ受信していない場合
（ACK 済みも含む）は `FND_E_BAD_ARG` を返し、IRQ の状態を変えません。
`IRQ` capability がない場合は引数にかかわらず `FND_E_ACCESS` を返します。

edge latch の性質上、ACK から W1C までの間に発生した要因は失われることがあります。そのためサービスは、**`irq_ack` の後にデバイスの状態を再確認しなければなりません**。例えば IRQ3 では、入力 FIFO が空になるまで読みます。

カーネルの IRQ 処理は、`PENDING & ENABLE` が 0 になるまで繰り返します（[IRQC §5](spec_IRQC.md)）。

### 10.3 MMIO の付与

付与できるのはバスマスタでないデバイスの窓だけで、範囲は byte 単位です。

| 範囲 | デバイス |
|---|---|
| `0x80000000–0x8000FFFF` | VDP |
| `0x80010000–0x8001FFFF` | SGC |
| `0x80020000–0x8002FFFF` | SGU |
| `0x80040000–0x8004002F` | PeC の UART と RNG |
| `0x80040030–0x8004017F` | PeC の入力 |
| VRAM、PCMRAM | 同時に付与できるのは 1 プロセスだけ |

- 上の範囲外を含む付与は `FND_E_ACCESS` になります。特に次のものは付与できません。
  - VPU、DMAC、BMC（いずれもバスマスタ）
  - PeC の timer、RTC、IDC
  - 拡張カード（DMA を持ちうる）
  - CPU 予約領域
- `perm` は R=1、RW=3 のいずれかです（X は付与できない）。

### 10.4 `blk_io` と `fnd_blk`（24 bytes）

| Off | 名前 | 意味 |
|---:|---|---|
| 0 | `drive` | IDC slot 0〜3 |
| 4 | `op` | 1 READ、2 WRITE、3 IDENTIFY |
| 8 | `lba` | 開始セクタ |
| 12 | `count` | セクタ数 1〜128（IDENTIFY では無視） |
| 16 | `buf` | 転送先。READ と IDENTIFY では W、WRITE では R の権限が必要 |
| 20 | — | 予約。0 にする |

- IDENTIFY は `buf` に 16 bytes を書きます（順に type、sector_size、capacity、flags）。
- カーネルは要求を FIFO で 1 つずつ処理し、DMAC は ch0 だけを使います。
- 転送の間、呼び出したスレッドは `BLK_IO` でブロックし、他のスレッドが走ります。
- IDC のエラーは次のように対応させます：2 → `NO_MEDIA`、3 → `BAD_ARG`、4 → `READ_ONLY`、5 と 6 → `IO`。

## 11. 起動の引き渡し

1. BIOS が stage1（LBA0）を実行します（[BIOS §3](spec_BIOS.md)）。
2. stage1 は FNDBOOT 全体（2 MiB 以下）を `0x00200000` に読み込み、magic を確認します。どの LBA から読むかは FS の仕様が定めます。
3. stage1 はカーネルを `kernel_load` へコピーし、次の状態で `kernel_entry` へ分岐します：S モード、R1 = 起動ドライブ、R2 = `0x00200000`。

FNDBOOT のヘッダ（64 bytes）：

| Off | 名前 | 内容 |
|---:|---|---|
| 0 | magic | `0x464E4442`（`FNDB`） |
| 4 | version | 1 |
| 8 | size | イメージ全体の byte 数 |
| 12 | kernel_off | カーネルの位置 |
| 16 | kernel_size | カーネルの大きさ |
| 20 | kernel_load | `0x00010000` |
| 24 | kernel_entry | カーネルの入口 |
| 28 | module_count | 同梱モジュールの数 |
| 32 | module_table_off | モジュール表の位置 |
| 36–63 | — | 予約。0 にする |

モジュール表の各エントリ（48 bytes）：

| Off | 名前 |
|---:|---|
| 0 | offset |
| 4 | size |
| 8 | priority |
| 12 | caps |
| 16 | grant_off |
| 20 | grant_count |
| 24–31 | 予約 |
| 32 | `name[16]` |

`grant_off` は、`proc_spawn`（§9.2）の `grants` と同じ 12 bytes（`start`、`end`、`perm`）の付与を `grant_count` 個並べた配列の位置です（0 個なら 0）。オフセットはすべてイメージ先頭からの byte 数で、4-byte 境界に置きます。`name` は NUL で終わる ASCII です。

stage1 は、`kernel_load` = `0x00010000`、`kernel_size` ≤ `0xE0000`（§5.1 のカーネルイメージ領域）、`kernel_entry` がカーネルの範囲内であること、`size` = FS2 の `boot_size` であることも検査します。stage1 が失敗したときは、割り込み禁止のまま R1 = 失敗した段階（1 = IDENTIFY か superblock の読み込み、2 = superblock の検査、3 = FNDBOOT の読み込みか検査）で HALT します。

カーネルは同梱モジュールを表の順に spawn します。各モジュールには、argv = [name]、cwd = `/`、役割表なしの起動ブロックを与えます。表の最後は init でなければなりません。すべて spawn し終えたら、staging 領域を解放します。

## 12. C からの利用

- ABI ヘッダ `foundation/include/foundation/{abi.h,errors.h,ipc.h}` に、本書の番号と構造体を定義します。
- ラッパーは `cpt32_syscall(n, 0, a1, a2, a3, a4)` で書きます。
- ユーザープログラムは `tools/llvm-src32` の `--wax3 --pie`（どこにでも読み込める再配置可能な WAX v3）でビルドするのが標準です。固定アドレスにしたいときは `--protected` や `--origin` で origin とスタックを指定します。
- 起動コードの終端にある `SYSCALL 0` は Foundation では `sys_info` にあたり、終了処理になりません。Foundation 向けの起動コードは `proc_exit` を呼ばなければなりません。`tools/llvm-src32` の `--wax3`（`--os foundation`、既定）が作る WAX v3 の起動コードは、`main` の戻り値で `proc_exit` を呼びます。`--protected`（WAX v2）の起動コードは今も `SYSCALL 0` なので、v2 のプログラムは自分で `proc_exit` を呼んで終わる必要があります。
- 共有ライブラリは `--wal` で WAL として作り、使う側は `--import-wal` で WAL を指定して WAX v3 にします（[WAX v3 §6](spec_WAX_v3.md) の call ABI 1）。

## 13. sol 互換 ABI：`!abi_compat "Foundation"`

### 13.1 ディレクティブ

```
!abi_compat "Foundation"
!abi_compat "Foundation" 1
```

- このディレクティブは、プログラムがどの ABI を前提とするかをコンパイラに指示します。ABI 名は大文字小文字を区別します。
  - major を省略した場合は 1 です。
  - ABI 名は `"Foundation"` と `"bare"` の 2 つです。`"bare"` は従来の CPT32 ベアメタル ABI を表し、ディレクティブがないときの既定です。
  - それ以外の名前と未対応の major はコンパイルエラーです。
- 置き場所：ファイルのトップレベル（名前空間ブロックの外）で、そのファイル内の関数定義、トップレベルのコード、`!asm`、`!data`、`!db` のいずれよりも前に書かなければなりません。
- 複数ファイルでの宣言：メインファイルと `!include` したファイルのどれでも宣言できます。ただし、すべての宣言が同じ名前・同じ major でなければならず、食い違いはエラーです。
  - ライブラリが `"Foundation"` を宣言し、メインプログラムが `"bare"`（または宣言なし）の場合はエラーです。メッセージには、どのファイルがどの ABI を要求しているかを示します。
  - 宣言のないライブラリは、どちらの ABI でも使えます。
- sol VM（`sol_vm.py`）は `"Foundation"` のプログラムを実行できず、その旨のエラーを出します。コンパイルは可能です。
- LSP と VS Code 拡張は、このディレクティブを認識し、ABI 名の補完と上記のエラーの診断を提供しなければなりません（同梱のコピーも同期する）。

### 13.2 `"Foundation"` で変わるコード生成

sol の言語の意味と、関数の呼び出し規約・フレーム（R26 のフレームベース、R28 のデータスタック）は変わりません。変わるのは、プログラムの外側との境界だけです。

| 項目 | `"bare"` | `"Foundation"` |
|---|---|---|
| 実行モード | S | U（PROTECTED WAX） |
| 起動時の R28 | `!stack_top` の値を設定する | カーネルが設定した値をそのまま使う（`!stack_top` は WAX の `user_stack_top` として出力する） |
| IRQ | `irq0`〜`irq15` がハンドラになる | `irq0`〜`irq15` という名前の関数はエラー |
| R30 | IRQ ディスパッチャ用に予約 | ユーザーの GP（UGP）。生成コードは変更しない |
| プログラムの終端 | `HALT` | `0 fnd_proc_exit` 相当（`SYSCALL 0x11`、R1 = 0） |
| `!asm` 内の `HALT`、`IRET`、`MFSR`、`MTSR` | 使える | エラー（行頭の命令名で判定する。安全のための保証ではなく、誤用を防ぐための検査） |
| 既定の配置 | origin 0、rodata `0x20000`、変数 `0x100000`、スタック 1 MiB | origin は旧スロット 5 の `0x00800000`。rodata はコード直後、変数は rodata 直後（いずれも 4-byte 境界）。スタック頂上は旧スロット 5 の末尾 `0x00C00000`、スタック容量は既定 256 KiB（`!force_stack_size` で変更できる） |
| 出力 | 従来の WAX または raw | PROTECTED WAX。ブロックの権限はコードが RX、rodata が R、変数が RW。ヒープはスタックの下端を越えない |

- 明示した `--org`、`!rodata_base`、`!var_base`、`!stack_top` は既定より優先します。ただし配置が `0x00C00000` までに収まらない場合は、コンパイル時にエラーにします。
- solc は権限とスタックを `tools/asm/asm.py` の指令で渡します。`.PROTECTED stack_top, stack_size` で PROTECTED WAX を選び、`.PERM rx`／`.PERM r`／`.PERM rw` で以降のブロックの権限を指定します（[CPU 仕様 §7.1](spec_CPU.md)）。
- 省略可能な major は、ディレクティブと同じ行に書かなければなりません。次の行の数値はトップレベルのコードです。

### 13.3 標準ライブラリ `sol/foundation/fnd.sol`

- このライブラリは `!abi_compat "Foundation"` を宣言しています。§4 のエラー定数（`FND_E_*`）、§8〜§10 の構造体オフセット（`FND_MSG_OP` など）、§9 の全システムコールのラッパーを提供します。名前は C 側と同じく `fnd_` で始めます（例：`fnd_ipc_call`、`fnd_sleep`、`fnd_proc_exit`）。
- 各ラッパーは、§7 の「R1 以外のレジスタは保存される」という規則に依存しています。引数はフレームから R1〜R4 に読み込み、`SYSCALL n` を実行し、R1 をローカルに保存して `ret` で返します。
- 高水準の C libfnd を使う場合は `sol/foundation/libfnd.sol` と `tools/solc/build_foundation.py` を使います。syscall ラッパーと異なり、C 呼び出し用の stack ABI へ変換し、status と出力引数をそのまま返します。[ライブラリの契約](../sol/foundation/README.md) と [親子 sol のサンプル](../examples/foundation-sol/README.md) を参照してください。

```
fn fnd_sleep (ticks) :
    local r 0
!asm
    LD R1, [R26 + 12]      ; ticks（ローカル 1 個の後の arg0）
    SYSCALL 0x0C
    ST R1, [R26 + 8]       ; r
!end
    r ret
;
```

- **成功が 0 であることと、sol の真が 0 であることは一致します**。そのため、`status if ... end` の then 節は成功時に実行されます。ただし、ハンドルや byte 数を返す呼び出しは、成功時に 0 以上の値を返します。これらの結果は `0 lt`（負なら真 = 0）で判定してから使ってください。

```
!abi_compat "Foundation"
!include "foundation/fnd.sol"

10 fnd_sleep if
    42 fnd_proc_exit       # sleep が成功（0）したとき
end
1 fnd_proc_exit
```

- 構造体は `!var` の領域か、ヒープ（`fnd_mem_grow`）に置き、`st` と `ld` を語単位で使って組み立てます。sol の `st` と `ld` は big-endian の 32-bit で、本書の構造体の配置と一致します。

### 13.4 C との相互運用

- sol のプロセスと C のプロセスは、§8 の IPC と §9.3 の起動ブロックを通じて対等に通信します。sol で書いたサービスを C のクライアントから使うことも、その逆もできます。
- sol と C の関数を直接呼び合うこと（リンク）は、v1 では定めません。呼び出し規約が異なるためです。WAX v3 の WAL を介した呼び出しには sol 側の ABI 1 アダプタが必要で、v3 の実装と一緒に定めます。

## 14. 適合確認（実装時に追加するテスト）

以下は将来の実装に対する要求で、実行済みのテストではありません。

- **トラップ**：U からの IRQ、SYSCALL、fault のそれぞれで、frame の保存と復元を往復したとき、全レジスタ、USP/UFP/UGP、PSM（Short モード）が保たれる。カーネル内の例外では `kernel_panic` が呼ばれる。
- **プリエンプト**：U モードの 2 つのタスクが tick によって交互に進む。片方が保護違反を起こしても、もう片方は動き続ける。
- **領域**：プロセスを切り替えた後、前のプロセスの範囲へのアクセスが保護違反になる。8 領域ちょうどの構成は受け付け、9 領域になる構成は `REGION_LIMIT` で拒否する。
- **ポインタ検証**：範囲外、カーネル領域、R のみの範囲への書き込み、オーバーフローする範囲のいずれでも `FND_E_ACCESS` になり、メモリは変化しない。
- **IPC**：
  - 要求の切り詰めが起き、`total` で検出できる。
  - 応答が容量を超えると `FND_E_LIMIT` になり、token は有効なまま残る。
  - ハンドルを転送すると、送信側でそのハンドルが閉じられる。
  - サーバーが終了すると `call` が `PEER_GONE` になる。
  - クライアントが終了すると `CLIENT_GONE` が 1 回だけ届く。
- **終了処理**：READ の途中で kill しても、転送が完了または中止するまでメモリが解放されない。spawn と exit を繰り返しても、ハンドル、アリーナのページ、カーネルヒープが漏れない。
- **sol**：
  - `!abi_compat` の不一致、未知の名前、配置違反、`irqN` 関数、`!asm` 内の特権命令が、すべてエラーになる。
  - 生成した WAX が PROTECTED で、権限とスタックが §13.2 のとおりになっている。
  - `fnd.sol` のラッパーで、`sleep` と `proc_exit` の結果が testbench 上で確認できる。
