# LLVM → SRC32 freestanding C / C++ toolchain

`csc` と独立した、C / C++ / LLVM IR → SRC32 ASM → WAX コンパイラ。
`tools/csc` は非推奨 (deprecated) です。今後更新しません。C guest の新規開発にはこの LLVM/SRC32 ツールチェーンを使用してください。
CPT32 の RAM 上で起動し、引数なしの `int main(void)` を呼び、戻り値を R1 に残して HALT します。

`<cpt32/cpu.h>` の `cpt32_halt()` はSRC32の`HALT`命令を出力するintrinsicです。引数・戻り値はなく、有効なIRQが受理されると呼出し直後から再開します。この関数名は予約済みで、ユーザー定義しないでください。inline assembly全般が使えるわけではなく、引き続き拒否されます。
拡張P（特権レベルとSYSCALL）用のintrinsicとSYSCALLハンドラーは下の「Extension P」を参照してください。

CMake 用モジュールは `cmake/SRC32.cmake`。

## 必要なツール

- Python 3.11 以降、`python -m pip install -r tools/llvm-src32/requirements.txt`
- Clang（検証済み: ROCm 同梱 Clang 20）。`--clang PATH`、環境変数 `CLANG`、PATH の順に選択。Windows では ROCm の標準インストール先も探索。
- CMake 3.20 以降と Ninja（CMake ビルド時）。Ninja がない場合は `python -m pip install ninja`。
- 実行検証用: `cargo build --bin src32_testbench`。Windows では `.exe` を使う。拡張子なしの古い Linux バイナリと混同しないこと。

Python と llvmlite は同じ環境へインストールしてください。複数 Python がある場合、CMake の `SRC32_PYTHON_EXECUTABLE` を明示します。
llvmlite は LLVM 自身による IR のパース・検証・モジュールリンク・データレイアウト計算に使用します。

## CMake

リポジトリルートから、付属の4翻訳単位の検証プロジェクトをビルド:

```sh
cmake -S tools/llvm-src32/examples/conformance -B build/src32-c -G Ninja -DSRC32_PYTHON_EXECUTABLE=/path/to/python -DSRC32_CLANG=/path/to/clang -DSRC32_OPTIMIZATION=1
cmake --build build/src32-c --target conformance
```

生成物は `build/src32-c/conformance.wax`。パスに空白があっても引数は分離して渡されます。
Ninja が PATH にない場合は `-DCMAKE_MAKE_PROGRAM=/path/to/ninja` も指定します。

自分のプロジェクト:

```cmake
cmake_minimum_required(VERSION 3.20)
project(my_cpt32_program LANGUAGES NONE)
include("/path/to/src32/tools/llvm-src32/cmake/SRC32.cmake")

src32_add_library(game_logic
    SOURCES src/game.c src/math.c
    INCLUDE_DIRECTORIES include
    DEFINITIONS SCREEN_WIDTH=320)

src32_add_executable(game
    SOURCES src/main.c
    LIBRARIES game_logic
    OPTIMIZATION 1
    ORIGIN 0
    STACK_TOP 0x100000
    ENTRY main)
```

- `src32_add_library` は再利用可能な **ソースライブラリ**。ネイティブ `.a/.lib` アーカイブではありません。`LIBRARIES` で依存するソースライブラリも指定可能。重複ソースは一度だけコンパイル。
- include ディレクトリ・define は利用側も含めて共有。ターゲットごとの private コンパイル設定や既存のネイティブライブラリを混在させる機能はありません。
- `src32_add_executable` は `<target>.wax` を生成する CMake カスタムターゲット。`SRC32_IMAGE` ターゲットプロパティから出力パスを取得可能。
- Clang の依存ファイルから、実際に読み込んだヘッダー変更を追跡。コンパイラ本体・ランタイム・アセンブラ変更でも再ビルド。
- `LANGUAGES NONE` は意図的です。ホスト用 C コンパイラの検出・リンクを行いません。このモジュールは標準 `add_executable()` の置換や既存 CMake プロジェクトの無変更移植用 toolchain file ではありません。
- 対応ジェネレータの検証対象は Ninja。各ターゲットは全翻訳単位をまとめてリンクするため、変更時にその実行ファイル全体を再コンパイルします。

## CLI

```sh
python tools/llvm-src32/driver.py main.c helper.c -I include -DLEVEL=3 -O1 -o game.wax
python tools/llvm-src32/driver.py main.c helper.c -S -o game.a
python tools/llvm-src32/driver.py main.c helper.c --emit-llvm -o game.ll
python tools/llvm-src32/driver.py game.ll -o game.wax
```

入力は `.c`、`.cpp`、`.cc`、`.cxx`、`.ll`、`.bc` の組合せ。C は C11、C++ は C++17 としてコンパイルします。`-O0`（既定）、`-O1`、`-O2` は C/C++ 入力に適用。IR 入力を再最適化する指定ではありません。
`--origin`、`--stack-top`、`--entry` で配置・起動関数を指定。
`--save-temps DIR` は各入力の IR、リンク済み `linked.ll`、`output.a` を保存。
`--depfile PATH` は出力生成成功後に Make 形式の依存ルールを書き出します。
`.bin` 出力は raw バイト列で配置・エントリ情報を持ちません。非ゼロ origin では WAX を使ってください（ROM image のように読み込む側が配置を知っている場合を除く）。
`--data-origin ADDR`（CMake `DATA_ORIGIN`）は ROM 用配置です。LLVM の `constant`（`const` 修飾の文字列・表など）はコードの後ろに残し、書込み可能な静的データは RAM の `ADDR`（8-byte 整列）に置きます。初期値を持つ変数を先に並べ、その初期値イメージを定数の後ろへ格納し、起動コードが `main` より前に word 単位でコピーし、続くゼロ初期化領域を 0 で埋めます。出力イメージに RAM 側の block は含まれません。`--protected` とは併用できず、起動時 SP が書込み可能データ領域に入る配置は拒否します。利用例は [C 版 BIOS](../../bios/README.md)。

## Extension P（特権レベルと SYSCALL）

[拡張P](../../doc/spec_CPU_priv.md) のための intrinsic を `<cpt32/cpu.h>` で宣言しています。すべて呼出し位置へ直接展開され、同名の関数は定義できません。

| intrinsic | 展開 | 制約 |
|---|---|---|
| `cpt32_syscall(number, context, a1, a2, a3, a4)` | a1〜a4 を R1〜R4 に置き `SYSCALL number, context`、R1 を返す | number・context は0〜255の整数定数。U/Sどちらでも使用可。`CPT32_SYSCALL0..4` マクロあり |
| `cpt32_mfsr(sr)` / `cpt32_mtsr(sr, value)` | `MFSR` / `MTSR` | sr は `CPT32_SR_*` の定数。Sモードのみ |
| `cpt32_run_user(entry, user_stack_top)` | カーネルの SP/FP/再開PC/STATUS を保存し、`USP`・`EPC`・`STATUS.PU` を設定して `IRET` | Sモードのみ。ユーザーのIEはカーネルと同じ。保護領域は事前に設定。再入不可 |
| `cpt32_return_to_kernel(code)` | 保存したカーネル状態へ `IRET` し、`cpt32_run_user` が code を返す | SYSCALLハンドラーから呼ぶ。戻らない |
| `cpt32_halt()` | `HALT` | Sモードのみ |
| `cpt32_jump(entry, r1, sp)` | R1=r1、R28(SP)=sp、R29(FP)=0 を設定して `JR entry` | 戻らない。モード・IEは変えない。ブートローダーの引き渡し用 |
| `cpt32_ctx_resume(frame)` | `cpt32_ctx_current = frame`、trap depth = 0 にしてトラップスタブの復元経路へ分岐 | `kernel_trap` が必要。Sモードのみ。戻らない。最初のコンテキストの開始用 |

`void syscall_dispatch(cpt32_trap_frame *frame)` を定義すると、`main` の前に SYSCALL ベクター（`0xFFFF000C`）を登録します。
入口は S のスタック（Uからの呼出しでもバンク切替えでカーネルの R28）に136 byteを確保し、`frame->r[0]` に CAUSE、`frame->r[n]` に Rn（`r[28]` は未使用で0）を保存して呼び出します。
戻ると `frame` から R1〜R31 を復元して `IRET` するので、`frame->r[1]` に書いた値が `cpt32_syscall` の戻り値になります。
番号は `CPT32_CAUSE_SYSCALL_NUMBER(frame->r[0])`、コンテキストは `CPT32_CAUSE_SYSCALL_CONTEXT(...)` で取り出します。

サンプル: [`examples/syscall-user-c`](../../examples/syscall-user-c/README.md)（カーネルとユーザープログラムを1つの WAX にまとめたもの）。

#### 文脈切替トラップスタブ（`kernel_trap`）

`cpt32_ctx_frame *kernel_trap(cpt32_ctx_frame *frame)` と `void kernel_panic(uint32_t cause, uint32_t epc)` を定義すると、起動コードは IRQ slot 0〜15（`JR R30`、S バンクの R30 = スタブ）と例外ベクター `0xFFFF0004`/`08`/`0C` をすべてスタブへ向け、IRQC ENABLE を 0 にします。契約は [Foundation 仕様 §3](../../doc/spec_Foundation.md) です。

- トラップ時、スタブは全レジスタ・CAUSE・EPC・STATUS を `*cpt32_ctx_current`（34 語の `cpt32_ctx_frame`）へ保存します。U からなら `r[28..30]` は `USP`/`UFP`/`UGP`、S（idle など）からなら現在の R28〜R30 です。
- R28 を `--stack-top`（カーネルスタック頂上）、R29 を 0 にして、割込み禁止のまま `kernel_trap(frame)` を呼びます。戻り値の frame を `cpt32_ctx_current` にし、EPC、STATUS（PIE/PU/PSM のみ）、R1〜R31 を復元して `IRET` します。S frame へ戻るときは R28/R29 を frame から読み、R30 はスタブのアドレスのまま保ちます。
- 保護領域には触れません。別プロセスの frame を返す前に、カーネルが `cpt32_mtsr` で書き換えてください。
- `cpt32_trap_depth` は 1 から始まり、カーネル実行中は 1 です。この間のトラップ（`cpt32_ctx_resume` 前の SYSCALL やカーネル内の例外）は frame を書き換えず、`kernel_panic(cause, epc)` を呼びます。
- `cpt32_ctx_current` と `cpt32_trap_depth` はスタブが用意するので、C では `<cpt32/cpu.h>` の extern 宣言で参照します（定義するとエラー）。`irqc_dispatch`/`syscall_dispatch`/`--protected` とは併用できません。
- `cpt32_ctx_resume` 後、`main` のスタックは捨てられます（スタブは毎回 `--stack-top` から使います）。
- S モードのコードは R30 を変えてはいけません（このバックエンドは R30 を使いません）。

サンプル: [`examples/kernel-trap-c`](../../examples/kernel-trap-c/README.md)（協調切替、S モード idle からの脱出、タイマープリエンプション）。

### `--protected`（ユーザーモード実行ファイル）

```sh
python tools/llvm-src32/driver.py app.c --protected --origin 0x100000 --stack-top 0x200000 --user-stack-size 0x4000 -o app.wax
```

- 出力は [保護付き WAX](../../doc/spec_WAX.md)。コード block は R/X、データ（静的データとゼロ初期化領域）block は R/W、ユーザースタックは `--stack-top` の下 `--user-stack-size` byte（既定 0x4000、8の倍数）です。
- 起動コードは HALT できないので、`main` の戻り値を R1 に置いて `SYSCALL 0` を発行します。**`SYSCALL 0` を exit とするのはこのツールチェーンの規約**で、読み込む BIOS/OS が定義します（Foundation では 0 番は `sys_info` で、終了は `proc_exit` です。[Foundation §12](../../doc/spec_Foundation.md)）。
- `cpt32_halt`、`cpt32_mfsr`、`cpt32_mtsr`、`cpt32_run_user`、`cpt32_return_to_kernel`、`cpt32_ctx_resume`、`irqc_dispatch`、`syscall_dispatch`、`kernel_trap` はSモード専用なのでエラーです。デバイスは BIOS/OS が SYSCALL で提供します。
- 現在、保護付き WAX を起動できるのは BIOS の WAX loader（未実装）だけです。ホストの直接ロードは拒否します。
- CMake では `src32_add_executable(... PROTECTED USER_STACK_SIZE 0x4000)`。

### WAX v3 / WAL

[WAX/WAL v3](../../doc/spec_WAX_v3.md) の実行ファイルとライブラリを、[call ABI 1](../../doc/spec_WAX_v3.md#6-public-call-abi-1-src32-module-stack-abi)（引数はスタック、R30 = モジュールの origin）で作ります。読み込むのは Foundation カーネルです（[Foundation §9.5](../../doc/spec_Foundation.md)）。

```sh
# ライブラリ（WAL）。位置独立で、--origin は指定しない
python tools/llvm-src32/driver.py lib.c --wal --image-id my.lib --abi-version 1.2 --exports lib.exports -o lib.wal
# それを使う実行ファイル（WAX v3）。固定アドレス
python tools/llvm-src32/driver.py app.c --wax3 --import-wal lib.wal --origin 0x800000 --stack-top 0xC00000 -o app.wax
```

- `--exports FILE` は WAL が公開する記号の一覧です。1 行に 1 つ、関数は名前と [§6](../../doc/spec_WAX_v3.md) の signature（例：`fnd_write fn(u32,ptr,u32,ptr)->i32`）、オブジェクトは `名前 object` と書きます。`#` 以降と空行は無視します。一覧にない関数は外から見えません。
- signature の型は C の型と対応していなければなりません。`u32`/`i32` は 32-bit 整数、`u64`/`i64` は 64-bit 整数、`ptr` はポインタ、戻り値の `void` です（符号は LLVM IR に残らないので、`u32` と `i32` の区別は一覧の記述に従います）。`char`、`short`、`bool` の引数や戻り値、構造体の値渡し、可変長引数は境界を越えられないのでエラーです。`fnptr`（関数記述子）はまだ対応していません。
- `--import-wal WAL`（複数可）を指定すると、定義のない関数とグローバル変数のうち、その WAL が export しているものを import にします。使わなかった WAL は依存表に入りません。依存の `min_minor` は指定した WAL の `abi_minor` です。C 側は普通に `extern` で宣言して呼びます。
- import した関数の呼び出しは、記述子（8 bytes、RW データの末尾に置く）の 2 語を読み、呼び出し側の R30 をフレームに保存して、R30 を提供側の origin にしてから `JALR` し、戻ったら R30 を戻します。import した関数のアドレスを取ることはできません（記述子は値でなければならないため）。import したオブジェクトは、記述子の 1 語目のアドレスで読み書きします。
- WAL のコードは位置独立です。アドレスはすべて「オフセット + R30」で作り、分岐は相対か R30 から計算します。**WAL の静的データにアドレスを入れることはできません**（v3 にはロード時の再配置がないため。例：`static int *p = &x;`、文字列へのポインタの表、関数ポインタの表）。そうした初期値はエラーになるので、実行時に計算してください。コード中の文字列定数や関数のアドレスは使えます。
- ブロックはコード（RX）とデータ（RW、静的データと import 記述子）の 2 つです。WAL にはスタックと入口がなく、起動コードも入りません。
- `--wax3` の起動コードは `main` の戻り値を R1 に置いて Foundation の `proc_exit`（`SYSCALL 0x11`）を呼びます。`--os` は今のところ `foundation`（OS ABI 1.0）だけです。
- `--image-id` の既定は出力ファイル名（小文字）、`--abi-version` の既定は `1.0` です。WAX では `abi_major` はアプリケーションの契約の版です。
- 実例：[`foundation/tests/wal`](../../foundation/tests/wal)（WAL から WAL を import する例）、libfnd の WAL 化（`foundation/boot/build.py` がシェルと `/bin` のコマンドをそれを import する WAX v3 にする。export 一覧は `foundation/lib/fnd/fnd.exports`）。

## C++ 開発サンプル

[`examples/hello-cpp`](../../examples/hello-cpp/README.md) はクラスを使う複数翻訳単位と C SDK の混在プロジェクトです。CMake と CLI の両方でビルドできます。
C++ の起動関数は `extern "C" int main()` とし、C SDK ヘッダーも `extern "C"` で囲みます。
ドライバは例外・RTTI・threadsafe statics・cxa atexit を無効にし、コンストラクタ／デストラクタの alias 生成も抑制します。
標準 C++ ライブラリ、new/delete ランタイム、動的グローバル初期化、終了時デストラクタは提供しません。weak/COMDAT を生成するテンプレートや inline メソッド等も現行バックエンドでは利用できません。メソッドを別の `.cpp` に定義する例と詳しい制限はサンプルを参照してください。

## 対応範囲

| 機能 | 契約 |
|---|---|
| C のデータモデル | big-endian、ポインタ/int/long/size_t は32 bit、char は8 bit signed、short は16 bit、long long は64 bit |
| 整数 | LLVM i1–i32 と i64、符号付き／符号なし算術・比較・除算・剰余・シフト・型変換 |
| メモリ | グローバル、ゼロ初期化、文字列、配列、構造体、packed 構造体、GEP、グローバル／関数ポインタの再配置 |
| 関数 | 複数翻訳単位、同名 static の分離、再帰、直接／間接呼出、構造体引数と戻り値 |
| 制御フロー | 分岐、ループ、switch、select、並列コピーによる phi |
| スタック | 固定／動的 alloca、VLA、stacksave/stackrestore。最大アラインメント8 byte |
| ランタイム | memcpy、memmove、memset、memcmp、strlen。未解決のものだけ同梱実装をリンク。ユーザー実装を優先 |
| LLVM intrinsic | メモリ操作、整数 min/max/abs/bswap/ctpop/ctlz/cttz、符号なし飽和減算 `llvm.usub.sat`（i1–i32/i64）、`llvm.umul.with.overflow.i32`、32-bit三方向比較 scmp/ucmp、expect/assume、lifetime/debug 等の対応部分 |

Clang の `mips-unknown-none` / o32 は **big-endian 32-bit C の IR 生成用**。MIPS 命令は生成しません。
LLVM の `ccc` / `fastcc` と元ターゲット ABI ヒント `inreg` は、直接・間接呼出側と定義側を一貫して SRC32 のスタック渡しへ変換します。Clang が `-O1` / `-O2` で内部関数へ付ける `fastcc` も利用できます。`tail` は通常呼出として扱い、末尾呼出最適化は保証しません。
ホスト用 x86 の `.ll/.bc` を単に SRC32 として読み替えることは禁止し、互換性のない data layout はエラーにします。
複数モジュールをリンクする場合、data layout 文字列は完全一致が必須です。配置へ影響しない差だけの場合も安全優先で拒否します。

### 明示的に非対応

浮動小数点、SIMD、atomic、可変長引数、例外、inline asm、TLS、alias/ifunc、weak/comdat 等の未対応 linkage、`ccc` / `fastcc` 以外の calling convention、`musttail`、i33–i63/i65以上。
未対応命令・intrinsic・定数は位置情報を含むエラーにし、不正な命令へ置換しません。
LLVM IR 全仕様／C11 全仕様を実装したという意味ではありません。

既定ではstdio、malloc、ファイル、OS API は付属しません。標準ヘッダーも `stddef.h/stdint.h/stdbool.h/limits.h/string.h` の freestanding 用です。
`--newlib-source PATH`（CMake: `NEWLIB_SOURCE PATH`）を明示すると、別途用意したNewlibのヘッダーとportable Cソースを使用し、文字列・整数・ソート・nano allocatorを利用できます。Newlib本体は同梱・自動取得しません。対応API、ヒープ設定、取得方法、配布上の注意は[外部Newlibサンプル](../../examples/newlib-c/README.md)を参照してください。printf/stdio、ファイルI/O、locale、libmなどのNewlib全体は未対応です。
追加の API はCで実装して一緒にリンクするか、同じ ABI を守る SRC32 ランタイムとして用意してください。
外部 C ライブラリの組み込み例は [`Berkeley SoftFloat デモ`](../../examples/softfloat-c/README.md)。Release 3e のソースを同梱し、整数演算で binary32 を実装するライブラリを `-O2` でリンクします。
既存 sol / csc の関数呼出 ABI とはバイナリ互換ではありません。

## SRC32 ABI とメモリ配置

- R0 はゼロ、R28 は SP、R29 は FP、R31 は LR。IRQライブラリ使用時はR30がベクタの分岐先用に予約されます。その他のレジスタは caller-saved。拡張Pでは R28〜R30 がS/Uでバンクされます。
- スタックは下向き、呼出境界は8-byte整列。引数は呼出時 SP から順に4-byte単位で格納。i64 と aggregate は big-endian の連続 word。
- callee は旧 FP、LR を8 byte保存し、その位置を FP とする。第1引数は FP+8。SSA 値・ローカルは FP より下の独立スロット。
- 32-bit戻り値は R1。64-bit戻り値は low=R1、high=R2。8 byte超の aggregate 戻り値は LLVM の sret ポインタを使用。
- 既定 origin=0、stack top=0x100000。コードに続けて整列済み静的データを配置。ゼロ初期化領域も WAX に含む。
- 起動時 SP がイメージ内に入る配置は拒否。ただし再帰／VLAによる実行時スタック枯渇の自動検出はありません。十分な RAM とスタック余白を確保してください。
- 分岐・呼出は絶対アドレスをロードして JR/JALR。大きなコードでも相対16-bit距離へ依存しません。
- 全 SSA 値を spill する正しさ優先の実装。ネイティブ LLVM ターゲット並みのコード密度・実行速度は保証しません。

[`sdk/cpt32`](../../sdk/cpt32/README.md) のCライブラリを利用できます。`void irqc_dispatch(void)`がリンクされる場合は、起動時に全16 IRQベクタを登録します。共通入口は128-byteの8-byte整列フレームにSP以外の全レジスタを保存し、dispatcher呼出し後に復元して`IRET`します。SPはフレーム解放で復元します。通常のC関数をベクタへ直接置いたり、R30を任意のアセンブリで破壊したりしないでください。ライブラリ検証は`python sdk/cpt32/smoke/run.py --clang PATH`。

## 実行検証

```sh
python -m unittest discover -s tools/llvm-src32 -p 'test_*.py' -v
```

`CLANG` と `SRC32_TESTBENCH` で実行ファイルを指定可能。テストは独立した作業ディレクトリで実行し、リポジトリの BIOS／ディスク設定を引き継ぎません。
検証プロジェクトは成功時42を返します。失敗時は100番台=メモリ、200番台=整数、300番台=呼出／制御、400番台=64-bit整数。
O0/O1/O2 × Scalar/Cached/CachedRelaxed/Pipeline を検証します。ほかに phi の循環コピー、長距離分岐、非ゼロ origin、診断、ASM再アセンブル、CMakeのヘッダー再ビルドを検証。
最適化後も残る static helper の `fastcc` 呼出と、実際の Furnace v2 stream を使う C SGUB driver の O0/O1/O2 × Pipeline（IRQ有効化・PCM初期化・周波数・音量）も検証します。テストベンチの初期化用5000万サイクルでは Scalar の `-O0` による全PCM初期化が終わらないため、このストリーム検証は Pipeline を使用します。
SGUB の密な初回リセット処理は、未最適化では1フレームのCPU予算を超えます。このテストは `--irq-drain-cycles 40000000` を指定し、通常のフレーム実行後に最大4000万サイクルだけ IRQ 復帰を待ってから毎フレームのメモリを検査します。待機中はVSyncを追加せず、VDPもtickしません。これは処理完了後の互換性検証であり、実時間再生の速度保証ではありません。

手動実行は **BIOSを設定していない作業ディレクトリ** から:

```sh
/path/to/src32_testbench /absolute/path/conformance.wax 42 --cpu pipeline
```

`src32_testbench --skip-render-checks` は R1 の期待値検査を省くため、この結果検証には使いません。
`--irq-drain-cycles N` は既定では無効です。有効時はCPUのIRQ許可状態が復帰するまで追加実行し、上限に達したらエラーにします。IRQを禁止したままの処理や復帰しないハンドラも成功扱いにしません。通常のフレーム予算での検証には指定しないでください。

## 根拠

- [SRC32 ISA](../../doc/spec_CPU.md)
- [WAX形式](../../doc/spec_WAX.md)
- [LLVM Language Reference](https://llvm.org/docs/LangRef.html)
- [llvmlite ModuleRef](https://llvmlite.readthedocs.io/en/latest/user-guide/binding/modules.html)
- 旧調査: [LLVM IR→SRC32 計画](../../doc/llvm_ir_to_src32_plan.md)
