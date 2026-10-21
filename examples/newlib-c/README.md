# 外部 Newlib を使う C ゲスト

Newlib は別途用意します。このリポジトリには Newlib のソース・ヘッダー・バイナリを同梱せず、自動ダウンロードもしません。
現在は LLVM/SRC32 の整数中心の機能に合わせた限定プロファイルです。**Newlib 全体の移植ではなく、printf、stdio、ファイルI/O、strtol、locale、libm は未対応です。**

## 用意するもの

[公式配布元](https://sourceware.org/pub/newlib/)からソースを取得し、リポジトリ外の任意の場所に展開してください。
検証版は `newlib-4.5.0.20241231.tar.gz`（SHA-256: `33f12605e0054965996c25c1382b3e463b0af91799001f5bb8c0630f2ec8c852`）。
指定するルートには `COPYING.NEWLIB` と `newlib/libc/` が必要です。
ほかに [LLVM/SRC32 ツールチェーンの必要ツール](../../tools/llvm-src32/README.md#必要なツール)を用意します。

```sh
python tools/llvm-src32/driver.py examples/newlib-c/main.c --newlib-source /path/to/newlib-4.5.0.20241231 -O1 -o newlib_c.wax
```

PowerShell でも同じ引数が使えます。空白を含むパスは引用符で囲んでください。

```sh
cmake -S examples/newlib-c -B build/newlib-c -G Ninja -DCPT32_NEWLIB_SOURCE=/path/to/newlib-4.5.0.20241231 -DSRC32_PYTHON_EXECUTABLE=/path/to/python -DSRC32_CLANG=/path/to/clang
cmake --build build/newlib-c
```

BIOSやディスク設定のない別の作業ディレクトリから、絶対パスで実行します。

```sh
/path/to/src32_testbench /path/to/src32/build/newlib-c/newlib_c.wax 42 --cpu pipeline
```

サンプルは文字列・メモリ操作、qsort/bsearch、符号付き整数除算、malloc/calloc/realloc/freeを検証し、成功時42を返します。
callocの乗算オーバーフロー、容量超過のENOMEM、realloc失敗時のデータ保持、free領域の再利用も検査します。

## 対応する API

- `string.h`: memcpy、memmove、memset、memcmp、memchr、strlen、strcpy、strncpy、strcat、strncat、strcmp、strncmp、strchr、strrchr、strstr、strspn、strcspn、strpbrk
- `stdlib.h`: abs、labs、llabs、div、ldiv、lldiv、qsort、bsearch、malloc、calloc、realloc、free
- `malloc.h`: malloc_usable_size（nano allocatorが内部でも使用）
- `errno.h`: 単一実行コンテキストのerrno。allocatorがENOMEMを設定します。

宣言は外部Newlibヘッダーを使います。ヘッダーに存在するほかの関数が利用可能という意味ではありません。未対応の関数やLLVM命令はコンパイル時エラーになります。

ドライバは未解決関数に応じて、外部ソースのportable C実装をLLVM IRへ変換してリンクします。MIPS用libc.aなど既成の機械語アーカイブは使えません。
アプリケーションは指定されたO0/O1/O2で、NewlibソースはO0・サイズ優先経路でコンパイルします。既存のユーザー定義文字列関数は優先されます。
nano allocatorの関数群は一緒にリンクするため、一部だけをユーザー定義allocatorに差し替えることはできません。

ヒープは8-byte整列の静的配列で、既定64 KiBです。`--newlib-heap-size 131072`で変更できます（8以上、8の倍数）。
配列もWAXデータに含まれ、既存のイメージと初期スタックの重なり検査の対象です。必要に応じて`--stack-top`も変更してください。
実行時のスタック伸長検査はなく、十分なスタック余白が必要です。ヒープ容量は固定され、`_sbrk_r`相当のポートはその配列内だけを移動します。
単一実行コンテキスト用であり、IRQハンドラー内からallocatorを呼び出さないでください。

CMake側では次のように明示的に有効化します。`NEWLIB_SOURCE`を指定しないターゲットの挙動は従来どおりです。

```cmake
src32_add_executable(app SOURCES main.c
    NEWLIB_SOURCE "/path/to/newlib-4.5.0.20241231"
    NEWLIB_HEAP_SIZE 65536)
```

外部のソースと読み込んだヘッダーもdepfileで追跡します。ソースの変更後は再ビルドされます。

## ライセンスと配布

ここで同梱するのはCPT32側の設定・接続コード・サンプルだけです。Newlibは利用者が別途取得してください。
Newlibをリンクしたゲスト成果物を配布する場合は、取得した版の`COPYING.NEWLIB`および使用した各ソースのライセンス表示・配布条件を確認して、必要な表示を成果物に添付してください。
別途取得する方式でも、リンク後の成果物からNewlibのライセンス条件がなくなるわけではありません。

## 回帰テスト

環境変数`SRC32_NEWLIB_SOURCE`に外部ソースの絶対パスを設定して実行します。

```sh
python -m unittest discover -s tools/llvm-src32 -p 'test_*.py' -v
```

未設定時は外部ソースを必要とするテストをskipし、取得は行いません。設定時はO0/O1/O2それぞれでゲストを構築し、Scalar/PipelineでR1=42を確認します。
