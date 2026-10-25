# Foundation OS 実装案

Status: **実装案（未実装）**、2026-10-22。本書は、CPT32 上で動かす OS「Foundation」の構成・契約の候補・開発順序をまとめたものです。規範となる仕様ではありません。FSとVFSは [spec_FS2.md](spec_FS2.md) で策定しました。カーネル ABI・IPC・syscall・トラップは [spec_Foundation.md](spec_Foundation.md) で確定しました。番号・構造体・規則が本書と異なる場合は、仕様の方が優先します。各フェーズで、`doc/spec_Foundation*.md` などの仕様を別途確定させてから実装します。

名前には「すべての基礎になる」という意味を込めています。

## 1. 方針

| 項目 | 方針 |
|---|---|
| カーネル | マイクロカーネル。カーネルに置くのは、スケジューリング、プロセス／スレッド、メモリ所有権と Extension P の領域、IPC、IRQ、DMA を伴うデバイス操作だけにする |
| スケジューリング | プリエンプティブ。固定優先度とし、同じ優先度の中ではラウンドロビンにする |
| API | アプリには DOS 風の分かりやすい独自 API を提供する。POSIX 互換は目指さない（fork、signal、errno、link は導入しない） |
| 名前空間 | Unix 風の単一ルート。区切りは `/`。ドライブ文字は使わない |
| ファイルシステム | [FS2 (File System 2)](spec_FS2.md)。階層・256文字名・UID権限・ミリ秒時刻を持つ。DSFS は互換用に読めるようにする |
| シェル | DOS の知見（`examples/dos-c`、`dos.sol`）を生かして新しく作る |
| 実装言語 | カーネルは C11。サービス・ライブラリ・アプリは C11 または C++17（freestanding） |
| 重視する点 | 分かりやすさ。機能より契約の明確さを優先する |

## 2. 前提となる既存の資産と制約

| 資産／制約 | Foundation への影響 |
|---|---|
| [Extension P](spec_CPU_priv.md)：S/U の 2 モード、`SYSCALL imm8`、R28–R30 のバンク、8 つの保護領域 | 保護領域は CPU に 8 つしかない。プロセスを切り替えるたびに 8 領域すべてを書き直す。MMU はないため、全プロセスが一つの物理アドレス空間を共有する |
| 保護領域は DMA を検査しない | DMAC・BMC・IDC・VPU（バスマスタ）を U モードへ渡さない。ディスク転送はカーネルが検証してから実行する |
| [IRQC](spec_IRQC.md)：16 本の入力、固定優先度、edge latch | IRQ はすべてカーネルが受け、サービスへの通知に変換する。level 型の要因（IRQ3 の PeC 入力など）は、サービスが ACK するまで ENABLE で mask する |
| IRQ0 はホストのフレームループ由来の VSync | 時刻やスケジューリングには使わず、表示同期の通知だけに使う |
| PeC quad timer（48 MHz 基準、periodic、IRQ4） | スケジューラの tick に使う。RTC alarm と IRQ4 を共有するため、PeC の pending で要因を区別する |
| [IDC](spec_IDC.md) と DMAC | 512-byte sector、FDD は 1 sector あたり約 108 ms。ディスク I/O の完了待ちでは必ずスレッドをブロックし、他のスレッドを走らせる |
| [BIOS](spec_BIOS.md)：LBA0 の 512 bytes を `0x0` にロードし、S モードで分岐、R1 = drive | Foundation のブートセクタ（stage1）がカーネルを含むブートイメージを読み込む |
| WAX v2 PROTECTED：固定アドレス、ブロックは最大 7 つ、スタックは region 0 | フェーズ 1〜6 のサービスとアプリはこの形式を使う |
| [WAX/WAL v3](spec_WAX_v3.md)：仕様のみで未実装。ABI 1 は現行の LLVM ABI と異なる | WAL（共有ライブラリ）は後のフェーズに回す。それまで API ライブラリは静的リンクのソースライブラリにする |
| `tools/llvm-src32`：inline asm・可変長引数・atomic・浮動小数点が使えない。C++ には new/delete・グローバル初期化・COMDAT がない | トラップ入口はツールチェーンに生成させる（§4.2）。`kprintf` は作らず、`kputs`／`kputhex` を使う。C++ は制限付きでアプリ向けに使える |
| `syscall_dispatch` のスタブは、1 つの S スタックに保存して同じ文脈へ戻る | スレッドを切り替えられないため、文脈を切り替えられる新しいトラップスタブが必要になる |

## 3. 全体構成

```text
 アプリ (WAX, U)          shell, dir, copy, ...
   │  libfnd: DOS 風 API（静的ライブラリ → 後に WAL）
   │  SYSCALL（IPC / スレッド / プロセス）
 ─┼─────────────────────────────────────────
 カーネル (S)  foundation.krn
   ├ trap / 文脈切替 / スケジューラ (PeC timer IRQ4)
   ├ プロセス・スレッド・ハンドル表
   ├ メモリ所有権表 + 保護領域プラン
   ├ IPC（コピー方式）・サービス登録
   ├ IRQ → 通知の変換
   ├ WAX ローダ（検証・コピー・領域付与）
   └ blk_io: IDC + DMAC（転送先を検証してから実行）
 ─┼─────────────────────────────────────────
 サービス (WAX, U)
   ├ init      起動設定を読み、サービスとシェルを起動
   ├ console   PCG/VDP の文字表示、キー入力（PeC の入力窓だけを付与）
   ├ disk      ドライブ単位のキュー・キャッシュ（blk_io 経由）
   └ fs        VFS, FS2, DSFS(互換), /dev, /dd, /os
```

### 3.1 リポジトリ配置案

```text
foundation/
├─ README.md
├─ include/foundation/   ABI 共有ヘッダ（syscall 番号、メッセージ、エラー）
├─ boot/                 stage1 ブートセクタ（tools/asm で 510 bytes 以下）
├─ kernel/               C11、S モード、raw image
├─ lib/fnd/              libfnd（DOS 風 API）
├─ services/{init,console,disk,fs}/
├─ bin/{shell,...}/      シェルと外部コマンド
├─ tools/                ブートイメージ packer、ディスクイメージ作成
└─ tests/                ホスト単体テスト、testbench シナリオ
tools/fs2/               FS2 のホストツール（mkfs / fsck / put / get / ls）
doc/spec_Foundation.md, spec_Foundation_API.md, spec_FS2.md （各フェーズで作成）
```

アロケータ・パス解決・FS2 のディスク構造処理などの純粋なロジックは、SRC32 の intrinsic に依存しない C11 で書きます。こうするとホストの C コンパイラでも単体テストできます。

## 4. カーネル

### 4.1 メモリ配置案（Main RAM は既定 16 MiB）

| 範囲 | 用途 |
|---|---|
| `0x00000000–0x000001FF` | stage1（BIOS がロード。起動後は解放する） |
| `0x00010000–0x000EFFFF` | カーネルイメージ（コード、データ、BSS） |
| `0x000F0000–0x000FFFFF` | カーネルスタック（BIOS の作業領域は引き渡し後に再利用できる） |
| `0x00100000–0x003FFFFF` | カーネルヒープ：TCB、ハンドル表、IPC の一時バッファ、ブートイメージの退避先 |
| `0x00400000–RAM 末尾` | ユーザープール。4 KiB 単位の所有権表で管理し、所有者のない領域を付与しない |

保護領域は byte 単位で指定できます。それでも 4 KiB に揃えることで、所有権表を単純にします。再利用する前には必ずゼロクリアします。RAM 容量の取得方法（レジスタで取れるか、それとも probe するか）は未確認で、フェーズ 1 で決めます。

### 4.2 トラップと文脈切替

- カーネルは**割り込み禁止で動き、カーネル内ではプリエンプトしない**。トラップ入口で `IE=0` になるので、それをそのまま使います。カーネルスタックは CPU ごとに 1 本だけ持ちます。
- ユーザー文脈は TCB の frame に保存します。保存するのは R1–R27、R31、USP／UFP／UGP、EPC、STATUS（PU、PIE、PSM）です。ブロックする syscall は、TCB に結果を書いてから別のスレッドへ切り替えます（continuation 方式）。この方式なら、カーネルスタックはスレッドごとに要りません。
- ツールチェーンの拡張案：`cpt32_trap_frame *kernel_trap(cpt32_trap_frame *cur)` を定義すると、全ベクタ（IRQ・例外・SYSCALL）のスタブを生成するようにします。スタブはグローバル変数 `cpt32_current_frame` が指す frame に保存し、**戻り値の frame から復元して** `IRET` します。プロセスが変わる場合は、復元の前にカーネルの C コードが `MTSR` で保護領域を書き直します。その手順は「全領域を無効化 → START/END を設定 → FLAGS を設定」で、WAX v3 §7 に従います。
- トラップ元が S（idle ループ）なら、frame に保存しません。idle は S モードのループで、`IE=1` にして `HALT` します（HALT は特権命令です）。
- U モードでの例外（保護違反・不正命令・特権命令）が起きたら、そのプロセスを終了させます。終了理由として CAUSE と EPC を記録し、`/os/proc` から見えるようにします。S モードでの例外は kernel panic 画面にします。
- 性能上の注意：JIT がネイティブ実行するのは S モードだけです。U モードは Cached インタープリタで動きます。

### 4.3 スケジューラ

- 優先度は 4 段階：`IRQ`（通知を受けるドライバのスレッド）> `SERVICE` > `APP` > `IDLE`。同じ段階の中ではラウンドロビンにします。
- tick は PeC timer ch0 の periodic で、既定は 10 ms（100 Hz、RELOAD = 480,000 cycles）。タイムスライスの既定は 1 tick（10 ms）です。tick 間隔は起動設定で変えられるようにします。
- スレッドの状態は `READY / RUNNING / BLOCKED(ipc, sleep, wait, blk_io) / DEAD` です。BLOCKED からの解除は、必ずカーネル内の一箇所の関数（`wake(thread, result)`）を通します。

### 4.4 ハンドルと IPC

- ハンドル表はプロセスごとに持ちます（初期案は 64 エントリ）。ハンドルの種類は `PORT`（受信権）、`CHANNEL`（送信権と、送信元を識別する badge）、`PROCESS`、`THREAD`、`IRQ`、`DISK`（blk_io の権限）です。ハンドル番号は、ライブラリのファイルハンドルとは別の名前空間です。
- メッセージは 32 bytes の固定長：`op`、`arg[5]`、`buf`、`len`。
  - `call(ch, req, rep)` は送信して応答を待ちます。`req.buf/len` は送るデータ、`rep.buf/len` はクライアント側の受信先です。
  - サーバー側は `recv(port, msg, data, cap)` で受け、`reply(token, msg, data, len)` で返します。
  - カーネルは、各バッファが「呼び出したプロセスの所有範囲、かつ必要な R/W 権限を満たす」ことを検証してからコピーします。1 回の転送の上限は 4 KiB で、それより大きいデータは libfnd が分割します。
  - プロセス間で共有メモリは使いません。保護領域の予算を使わずに済み、所有権も単純になるためです。
- 応答の `msg` には、ハンドルを 1 つ載せて転送できます。この仕組みで、fs は `/dev/con` を open したクライアントへ、console サービスへの CHANNEL を直接渡せます。
- 相手が終了したとき：
  - 待っていた `call` は `E_PEER_GONE` を返します。
  - サーバーには、終了したクライアントの badge 付きでカーネルから `NOTIFY_CLIENT_GONE` が届きます。fs はこれを受けて、そのクライアントのファイルを閉じます。
- サービス登録：カーネルが小さな名前表を持ちます。`svc_register(name, port)` は登録権限を持つプロセスだけが呼べ、`svc_lookup(name)` は CHANNEL を返します。初期版では、別の name server は作りません。

### 4.5 IRQ とデバイスの権限

| IRQ | 所有者 | 扱い |
|---|---|---|
| 0 VSync | カーネル → console への通知 | 表示の同期だけに使う |
| 1 DMAC, 2 IDC | カーネル | blk_io の完了処理 |
| 3 PeC 入力 | カーネル → console への通知 | 通知した時点で mask し、`irq_ack` で unmask する |
| 4 timer / RTC | カーネル | tick と sleep。RTC alarm は後で考える |
| 5–12 拡張カード | `irq_bind` したサービス | IRQ capability が必要 |

MMIO の付与は、起動設定で許可したサービスにだけ、byte 単位の最小範囲で行います。例えば console には PCG/VDP の窓と、PeC 入力レジスタ `0x100–0x17F` 相当だけを渡します。DMAC・BMC・IDC・VPU の窓は誰にも渡しません。VDP と PCG がバスマスタでないことは、フェーズ 2 で確認します。

### 4.6 システムコール案（`SYSCALL n`、引数は R1–R4、結果は R1）

| 番号 | 名前 | 番号 | 名前 |
|---:|---|---:|---|
| 0x00 | `sys_info`（ABI 版、/os 用の統計情報） | 0x10 | `proc_spawn`（メモリ上の WAX を渡す） |
| 0x01 | `ipc_call` | 0x11 | `proc_exit` |
| 0x02 | `ipc_recv` | 0x12 | `proc_wait` |
| 0x03 | `ipc_reply` | 0x13 | `proc_kill` |
| 0x04 | `ipc_reply_recv` | 0x14 | `proc_startup`（起動情報のコピー） |
| 0x05 | `port_create` | 0x18 | `mem_grow`（RW ヒープの伸長） |
| 0x06 | `handle_close` | 0x20 | `irq_bind` |
| 0x07 | `handle_dup` | 0x21 | `irq_ack` |
| 0x08 | `thread_create` | 0x28 | `blk_io`（DISK ハンドルが必要） |
| 0x09 | `thread_exit` | 0x30 | `time_get`（tick、RTC） |
| 0x0A | `yield` | 0x31 | `svc_register` |
| 0x0B | `sleep` | 0x32 | `svc_lookup` |
| 0x0C | `wait`（ハンドルの完了、timeout 付き） | 0x3F | `debug_write`（初期の開発用） |

- エラーは R1 に負の値で返します。共有の errno は使いません。
- `SYSCALL` の context byte は将来の ABI 版の識別用に予約し、当面は 0 とします。
- WAX v3 の入口は R1–R27 = 0 で始まります。そのため、引数・カレントディレクトリ・標準ハンドルは `proc_startup` でプロセス自身のバッファへコピーさせます。

### 4.7 プロセス起動

- WAX の検証・コピー・領域付与はカーネルで行います。所有権表と領域プランを持っているのがカーネルだからです。ファイルの読み込みは呼び出し側が担当します（シェルが fs から読み、そのバッファを `proc_spawn` に渡す）。
- 保護領域の割り当て：region 0 はスタック、1〜はコード（RX）とデータ（RW）、データの直後に RW ヒープを置き、必要なら MMIO を加えます。ヒープは `mem_grow` で伸ばし、データとヒープは隣接しているので 1 つの領域にまとめます。
- **固定アドレスの問題**：v2 と v3 の WAX はどちらも固定アドレスで、MMU もありません。そのため、同じアドレスにリンクされたアプリは同時に動かせません。
  - 初期案：サービスごとに固定のリンクアドレスを割り当てます（`foundation/include` の表で管理）。アプリは共通のアプリスロットに置き、重なったら `ADDRESS_CONFLICT` を返します。
  - 将来案：WAL と同じ R30 origin 方式で、再配置できる WAX にします（WAX v3 の拡張として別途仕様化）。

## 5. 起動の流れ

1. BIOS が LBA0（stage1）を `0x0` にロードして分岐する（R1 = drive）。
2. stage1 は FS2 の superblock を検証し、予約ブート領域の FNDBOOT（2 MiB 以下）を `0x00200000` に読みます。カーネルを `0x00010000` へコピーし、R1 = drive、R2 = `0x00200000` でカーネル入口へ分岐します（Foundation ABI §11）。
3. ブートイメージ `FNDBOOT` の構成は、カーネルの raw image と、同梱モジュールの表（`init`、`console`、`disk`、`fs` の v2 PROTECTED WAX）です。
   カーネルは次の順に初期化します。
   1. ベクタを設定する。
   2. IRQC を設定する。
   3. timer を設定する。
   4. 所有権表を作る。
   5. 同梱モジュールをメモリから起動する（最後に init）。
4. fs が起動ディスクを `/` にマウントします。init は `/etc/startup`（1 行に 1 コマンドの単純なテキスト）に従って、残りのサービスとシェルを起動します。

stage1 は署名 `0x55 0xAA` を保つため、DSFS のブートセクタ規約と矛盾しません。

## 6. 名前空間とファイルシステム

### 6.1 ルート構成

| パス | 実体 |
|---|---|
| `/bin`, `/lib`, `/etc`, `/home` | 起動ディスクの FS2 上のディレクトリ（`/` の実体は起動ディスクのルート） |
| `/dev` | 仮想。`con`、`kbd`、`null`、`uart0`、`rtc` など。open するとデバイスサービスへの CHANNEL が返る |
| `/dd/<n>/` | IDC slot n（0〜3、BIOS の dd0〜dd3 と同じ番号）の FS ルート。FS2 と DSFS を扱う（DSFS は初期版では読み取り専用、§6.3）。ホスト共有ディレクトリもIDCがFS2互換媒体として提供する（§6.4）。例：`/dd/0/example.txt`。媒体がないときはアクセス時に `E_NO_MEDIA` |
| `/os` | 仮想で、読み取り専用の情報。`proc/<pid>/{status,name,handles}`、`kernel/{version,memory,scheduler,uptime}`、`services/<name>/` |
| `/tmp` | RAM 上の一時 FS（fs サービス内） |

- `dev`、`dd`、`os`、`tmp` は予約名です。起動ディスクのルートに同名のエントリがあっても隠し、作成も拒否します。
- セクタの生アクセスは、ファイルとしては公開しません。DISK ハンドルを使う専用 API で行います。
- 管理操作（終了要求など）は、`/os` への書き込みではなくプロセス API で行います。
- 初期版では、シンボリックリンク・ハードリンク・mount コマンドを導入しません。UID権限はFS2仕様に従います。ユーザー分離を有効にする前に、現行カーネルABIにないUID認証経路を定めます。

### 6.2 FS2 設計の要点（[FS2仕様](spec_FS2.md)）

従来の仮称FDFSはFS2 (File System 2)へ置き換えました。本節と異なる場合はFS2仕様を優先します。実装はまだありません。

- DSFS由来の512-byte block、big-endian、ブート署名、MSB-firstビットマップを使います。
- block 0はブートセクタ、block 1はsuperblock。その後は予約ブート領域、ビットマップ、128-byte inode表、データ領域です。
- 空きはビットマップ、ファイル内の順序は各データblock先頭の次block番号で管理します。payloadは508 bytesで、非連続割り当てが可能です。
- ディレクトリは1040-byte固定長エントリのストリームです。名前はUTF-8、最大256 Unicode scalar values / 1024 bytes。保存時の大小文字を保ち、Unicode 16.0.0 full case foldingで照合します。
- 時刻は符号付き64-bit UNIX秒と0〜999のミリ秒部。権限はUIDとowner/otherのRWXだけを持ち、GIDはありません。
- VFSがパス、mount境界、資格情報、権限、ハンドルを一元管理します。DSFS、RAM、デバイス、OS情報も共通操作で扱います。
- 読み書きmount中はDIRTYを保持し、異常終了後は通常mountを拒否してfsckを要求します。ジャーナルはなく、任意の中断からの自動回復や内容保全は保証しません。
- ホストツール（配置予定 `tools/fs2/`）を先に作り、ゲスト側は読み取りから始めます。
- DSFS媒体は `/dd/<n>` に平坦なディレクトリとしてread-only mountします。既存の `sdk/cpt32` のDSFSコードを参考にします。

### 6.3 `/dd/` でのDSFS互換

既存の [DSFS媒体](spec_DSFS.md) はFS2へ変換せず、そのまま `/dd/<n>/` から参照できるようにします。これはVFSの互換ドライバによる公開で、DSFSのディスク形式を拡張するものではありません。以下は実装予定の契約です。

- **形式の識別と検証**：slotごとにsuperblockのmagic/versionを確認し、FS2またはDSFSのドライバで検証してから公開します。ブート署名 `55 AA` だけでは形式を判定しません。破損した媒体を別形式として読み直したり、自動formatしたりしません。
- **平坦な名前空間**：DSFSのファイルを直下に並べます。例として `HELLO.TXT` は `/dd/1/HELLO.TXT` で開け、`/dd/1/hello.txt` でも同じファイルを参照します。列挙では元の大文字8.3名を返します。DSFS内にサブディレクトリを合成せず、`/dd/1/..` はVFS側の `/dd` に戻ります。
- **共通API**：FS2と同じディレクトリ列挙・stat・open・read・seek・closeを使います。アプリやシェルがDSFS専用APIを選ぶ必要はありません。媒体なしは `E_NO_MEDIA`、不正構造は `E_FORMAT`、未対応versionは `E_UNSUPPORTED` として扱います。
- **初期版は読み取り専用**：書き込みopen、create、write、truncate、delete、rename、mkdir、rmdir、chmod、chownは `E_READ_ONLY` とし、UID 0にも書き込みを許可しません。DSFS自体に書き込み機能があっても、Foundationの初期互換ドライバでは公開しません。
- **属性の補完**：DSFSにないUID/modeはVFS上で補い、uid=0、ファイルmode=`0x0024`（owner/other R）、ルートmode=`0x002D`（owner/other RX）とします。32-bit UNIX秒は64-bitへゼロ拡張し、ミリ秒部は0にします。元媒体に属性を書き戻しません。実行権限は既定では付与しません（[FS2仕様 §11](spec_FS2.md)）。
- **FS2との受け渡し**：シェルのCOPYなどでDSFSのファイルをFS2へ読み出して保存できます。保存先の名前・UID・mode・時刻はFS2側の作成規則に従います。別mountへのrenameで変換を代用せず、DSFSをFoundationの `/` 用FS2起動媒体として扱いません。
- **受け入れ確認**：既存ツールで作ったDSFSイメージを使い、列挙・大小文字の異なる名前での読み出し・EOF・属性補完・`..`・媒体交換を確認します。拒否される更新操作の前後で媒体のbyte列が変わらないことも検証します。

### 6.4 ホスト共有ディレクトリのマウント

ホストが提供する共有ディレクトリをIDC slotへ接続し、その内容を `/dd/<n>/` で公開します。**ホストのディレクトリ階層・ファイル・属性をFS2互換のsector列へ変換する処理は、エミュレータのIDC内で実装します。** Foundationのfsサービスは通常のFS2ドライバを使ってマウントします。これは今後のIDC拡張であり、現行のraw image設定ではまだ利用できません。

```text
ホスト共有ディレクトリ（例：C:/share）
    → IDC内の共有ディレクトリbackend：FS2互換の仮想媒体へ変換
    → IDC slot n：IDENTIFY / READ / WRITE、512-byte sectors
    → kernel blk_io → diskサービス → FS2ドライバ → VFS
    → /dd/<n>/（例：C:/share/docs/readme.txt → /dd/2/docs/readme.txt）
```

- **ゲストからの見え方**：superblock・ビットマップ・inode・ディレクトリ・データ連結を含め、[FS2仕様](spec_FS2.md)のmount検証を通る媒体を提供します。IDENTIFYは仮想媒体のcapacityとread-only状態を返します。専用のホストパスAPIや別種のVFSドライバは設けません。
- **設定**：ホスト側の `config.yaml` で共有元とslotを指定する方式とし、raw image指定とは排他的にします。キー名・容量・アクセスモード等のスキーマはIDC拡張仕様で定めます。ゲストのパスからホストの任意ディレクトリを指定する仕組みにはしません。
- **変換できる名前**：UTF-8、最大256文字、大小文字を区別しないFS2の規則を適用します。同一親の `Readme` / `README` のような衝突、不正な名前、表現できない容量・属性は接続時に報告し、黙って改名・省略しません。ホストの大小文字比較規則をFS2の比較に代用しません。
- **共有範囲と属性**：共有元をルートとして扱い、symlink・junction等をたどって範囲外を公開しません。ホストのUID/GIDやACLをそのままゲストへ渡さず、FS2のUID/modeと64-bit秒＋ミリ秒へ対応付けます。対応規則、リンク等の非対応項目の扱いはIDC側で定めます。
- **更新と整合性**：公開中のsector列がホスト側の変更だけで不整合にならないよう、snapshot・再生成・媒体交換通知の規則をIDC側で定めます。ゲストから書込可能にする場合も、FS2の複数sectorにまたがる更新を途中の状態でホストのファイル操作へ反映してはなりません。書戻しの確定点、削除・改名、DIRTY、外部編集との競合、失敗時の扱いは実装前に仕様化します。read-onlyで提供する場合はIDCの書込禁止を通知し、WRITEを拒否します。
- **受け入れ確認**：同じ内容のFS2イメージと共有ディレクトリで、ゲストからの列挙・階層探索・読み出しが一致することを確認します。名前衝突、共有範囲外参照、属性変換、ホスト外部編集、再接続後の古いハンドルも検証します。書込対応時は中断・競合を含めたホストへの反映を別途確認します。

## 7. API ライブラリ libfnd とシェル

libfnd とサービスのプロトコルは [API 仕様](spec_Foundation_API.md)、シェルは [シェル仕様](spec_Foundation_shell.md) で定めました。以下と異なる場合は仕様が優先します。

- 名前は `fnd_` で始めます。戻り値は `fnd_status`（0 または負のエラー）で、結果は出力引数で返します。
  - ファイル：`fnd_file_open(path, mode, &h)`、`fnd_file_read`、`fnd_file_write`、`fnd_file_seek`、`fnd_file_close`、`fnd_file_delete`、`fnd_file_rename`。
  - ディレクトリ：DOS の FindFirst/FindNext 風の `fnd_dir_open`／`fnd_dir_next`／`fnd_dir_close`、`fnd_dir_make`、`fnd_dir_remove`。
  - カレントディレクトリ：`fnd_cwd_set`／`fnd_cwd_get`。fsがcwdの生存参照を保持し、libfndが絶対パスに変換して送ります。途中の `..` は消去せず、mount境界・型・探索権限の検査をVFSに任せます（FS2 §11）。
  - プロセス：`fnd_proc_exec(path, args, &ph)`、`fnd_proc_wait`。
  - コンソール：`fnd_con_write`、`fnd_con_read_line`、`fnd_con_getkey`。
  - 標準ハンドル：`fnd_std_in()`、`fnd_std_out()`（起動情報から取得する）。
- エラーコード（初期案）：`E_OK=0`、`E_NOT_FOUND`、`E_EXISTS`、`E_NOT_DIR`、`E_IS_DIR`、`E_NOT_EMPTY`、`E_BAD_PATH`、`E_BAD_HANDLE`、`E_ACCESS`、`E_NO_MEDIA`、`E_IO`、`E_NO_SPACE`、`E_NO_MEMORY`、`E_LIMIT`、`E_PEER_GONE`、`E_FORMAT`、`E_UNSUPPORTED`。
- シェル `/bin/shell.wax`（仕様は [spec_Foundation_shell.md](spec_Foundation_shell.md)。以下と異なる場合は仕様が優先。2026-10-22 にコマンド名を UNIX 風（`ls`、`cat`、`pwd` など）に変更し、`KILL` は v1 では除外）：
  - シェルはディスパッチャだけを受け持ちます（2026-10-22 決定）。内部コマンドは `cd`、`set`、`exit`、`help` だけで、`ls`、`cat`、`cp`、`pwd`、`echo`、`ps` などは `/bin/<name>.wax` の別の実行ファイルです。共通部分は libfnd を WAL にして共有します（フェーズ 7。それまでは静的リンク）。
  - コマンド名は大文字小文字を区別しません。内部コマンドでなければ、`/bin/<name>.wax` を探して起動します。
  - `/` はパス区切りなので、オプションには `-x` を使います（DOS の `/x` は使わない）。
  - 行編集とコマンドの解釈は `examples/dos-c` の実装を参考にします。

## 8. 開発フェーズと受け入れ確認

| フェーズ | 内容 | 受け入れ確認 |
|---|---|---|
| 0 ツール | 文脈切替トラップスタブ（§4.2）、カーネル raw image のビルド、`FNDBOOT` packer | ツールチェーンのテスト。スタブが別 frame へ切り替えて復元すること（トラップスタブは実装済み: `examples/kernel-trap-c`。raw image と packer も実装済み: `foundation/boot/build.py`、`fndboot.py`。stage1 とディスクイメージは `foundation/tests/disk` で確認） |
| 1 最小カーネル | trap、timer tick、U モードタスク 2 つの交互実行、例外での終了 | testbench で両タスクのカウンタが進み、片方が保護違反で落ちても他方が動き続ける（実装済み: [`foundation/`](../foundation/README.md)、`tests/phase1`。WAX ローダはないので、プロセスはカーネルイメージにリンクしたユーザーコードから作る。RAM 容量の取得は未決） |
| 2 IPC | ハンドル、call/recv/reply、相手終了時の解除、console サービス | console 経由の文字表示。サーバーが終了したら call が `E_PEER_GONE` になる |
| 3 プロセス | `proc_spawn`/`exit`/`wait`、所有権表、`FNDBOOT` の同梱モジュール | 起動・終了を繰り返しても RAM とハンドルが漏れない。WAX v3 §8 の領域ケースのうち v2 で可能なもの |
| 4 ディスク | `blk_io`（転送先の検証）、disk サービス、DSFS 読み取りマウント | FDD 読み込み中も他スレッドが進む。不正バッファでは `E_ACCESS` になり RAM は変化しない |
| 5 FS2 | 仕様、ホストツール、ゲストの読み取り → 書き込み | ホストの fsck とゲストの結果が一致する。書込中断後はDIRTYでmountを拒否し、fsckの検査・修復可能範囲を確認する |
| 6 シェル | libfnd、shell、外部コマンド、stage1 からのディスク起動 | ディスクから起動して `DIR`、`TYPE`、アプリ起動を確認する（fs なしで動く部分を先に実装済み: [`foundation/shell`](../foundation/README.md#シェル)。コンソールはシェルにリンクした暫定版で、PCG に表示する） |
| 7 拡張 | WAX v3／WAL（libfnd の WAL 化）、再配置可能な WAX、`/tmp`、拡張カードのドライバ | WAX v3 §8 の全ケース（ローダは実装済み: `foundation/kernel/wax.c`、`tests/wax`。v2 PROTECTED も同じ経路で読み込む。v3 を生成するツールチェーンと libfnd の WAL 化は未着手） |

各フェーズでは、テストに通った focused check と、実行していない確認を分けて報告します。

## 9. 決める必要がある点

| 論点 | 推奨 |
|---|---|
| アプリのアドレスモデル | 初期は固定スロット。将来は R30 origin で再配置可能な WAX |
| ファイル名の大文字小文字 | FS2仕様で確定：保存名を維持し、Unicode 16.0.0 full case foldingで比較する |
| ファイル名の文字コード | FS2仕様で確定：UTF-8、最大256 Unicode scalar values / 1024 bytes |
| WAX ローダの置き場所 | カーネル内（所有権表と原子的な起動を一箇所で扱うため） |
| tick とタイムスライス | 10 ms と 10 ms |
| ゲスト側の設定形式 | 1 行に 1 コマンドのテキスト。YAML はホスト側の設定にだけ使う |
| C++ の利用範囲 | カーネルとサービスは C11。アプリは C++17 も可（new/delete・グローバル初期化なし） |
