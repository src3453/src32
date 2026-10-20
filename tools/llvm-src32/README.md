# LLVM → SRC32 freestanding C / C++ toolchain

`csc` と独立した、C / C++ / LLVM IR → SRC32 ASM → WAX コンパイラ。
`tools/csc` は非推奨 (deprecated) です。今後更新しません。C guest の新規開発にはこの LLVM/SRC32 ツールチェーンを使用してください。
CPT32 の RAM 上で起動し、引数なしの `int main(void)` を呼び、戻り値を R1 に残して HALT します。

`<cpt32/cpu.h>` の `cpt32_halt()` はSRC32の`HALT`命令を出力するintrinsicです。引数・戻り値はなく、有効なIRQが受理されると呼出し直後から再開します。この関数名は予約済みで、ユーザー定義しないでください。inline assembly全般が使えるわけではなく、引き続き拒否されます。

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
`.bin` 出力は raw バイト列で配置・エントリ情報を持ちません。非ゼロ origin では WAX を使ってください。

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
| LLVM intrinsic | メモリ操作、整数 min/max/abs/bswap/ctpop/ctlz/cttz、符号なし飽和減算 `llvm.usub.sat`（i1–i32/i64）、expect/assume、lifetime/debug 等の対応部分 |

Clang の `mips-unknown-none` / o32 は **big-endian 32-bit C の IR 生成用**。MIPS 命令は生成しません。
LLVM の `ccc` / `fastcc` と元ターゲット ABI ヒント `inreg` は、直接・間接呼出側と定義側を一貫して SRC32 のスタック渡しへ変換します。Clang が `-O1` / `-O2` で内部関数へ付ける `fastcc` も利用できます。`tail` は通常呼出として扱い、末尾呼出最適化は保証しません。
ホスト用 x86 の `.ll/.bc` を単に SRC32 として読み替えることは禁止し、互換性のない data layout はエラーにします。
複数モジュールをリンクする場合、data layout 文字列は完全一致が必須です。配置へ影響しない差だけの場合も安全優先で拒否します。

### 明示的に非対応

浮動小数点、SIMD、atomic、可変長引数、例外、inline asm、TLS、alias/ifunc、weak/comdat 等の未対応 linkage、`ccc` / `fastcc` 以外の calling convention、`musttail`、i33–i63/i65以上。
未対応命令・intrinsic・定数は位置情報を含むエラーにし、不正な命令へ置換しません。
LLVM IR 全仕様／C11 全仕様を実装したという意味ではありません。

stdio、malloc、ファイル、OS API は付属しません。標準ヘッダーも `stddef.h/stdint.h/stdbool.h/limits.h/string.h` の freestanding 用です。
追加の API はCで実装して一緒にリンクするか、同じ ABI を守る SRC32 ランタイムとして用意してください。
外部 C ライブラリの組み込み例は [`Berkeley SoftFloat デモ`](../../examples/softfloat-c/README.md)。Release 3e のソースを同梱し、整数演算で binary32 を実装するライブラリを `-O2` でリンクします。
既存 sol / csc の関数呼出 ABI とはバイナリ互換ではありません。

## SRC32 ABI とメモリ配置

- R0 はゼロ、R28 は SP、R29 は FP、R31 は LR。IRQライブラリ使用時はR30がベクタの分岐先用に予約されます。その他のレジスタは caller-saved。
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
最適化後も残る static helper の `fastcc` 呼出と、実際の Furnace v2 stream を使う C SGUB driver の O0/O1/O2 × Pipeline（IRQ有効化・PCM初期化・周波数・音量）も検証します。

手動実行は **BIOSを設定していない作業ディレクトリ** から:

```sh
/path/to/src32_testbench /absolute/path/conformance.wax 42 --cpu pipeline
```

`src32_testbench --skip-render-checks` は R1 の期待値検査を省くため、この結果検証には使いません。

## 根拠

- [SRC32 ISA](../../doc/spec_CPU.md)
- [WAX形式](../../doc/spec_WAX.md)
- [LLVM Language Reference](https://llvm.org/docs/LangRef.html)
- [llvmlite ModuleRef](https://llvmlite.readthedocs.io/en/latest/user-guide/binding/modules.html)
- 旧調査: [LLVM IR→SRC32 計画](../../doc/llvm_ir_to_src32_plan.md)
