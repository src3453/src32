# Berkeley SoftFloat を C から使うデモ

Berkeley SoftFloat Release 3e の C ソースを LLVM/SRC32 用にビルドし、CPT32 ゲストから呼び出す外部ライブラリ利用例です。PCG の80桁画面に binary32 の結果ビット列と例外フラグを表示し、成功時は `PASS: all 14 checks`、R1=42 を残して HALT します。

四則演算、整数との変換、丸めモードによる `1/3` と `2.5` の違い、ゼロ除算、NaN、subnormal、符号付きゼロ、overflow/underflow を検証します。期待値は IEEE-754 のビット列と比較します。`float32_t.v` は整数のビット表現であり、C の `float` 演算や LLVM 浮動小数点命令は使用しません。

## ビルド

Python 3.11以降、Clang 20、CMake 3.20以降を用意し、リポジトリルートで実行します。PowerShell の例:

```powershell
python -m pip install -r tools/llvm-src32/requirements.txt ninja
$python = python -c 'import sys; print(sys.executable)'
$ninja = python -c 'import pathlib, ninja; print(pathlib.Path(ninja.BIN_DIR) / "ninja.exe")'
cmake -S examples/softfloat-c -B target/softfloat-c -G Ninja `
  "-DSRC32_PYTHON_EXECUTABLE=$python" `
  '-DSRC32_CLANG=C:/Program Files/LLVM/bin/clang.exe' `
  "-DCMAKE_MAKE_PROGRAM=$ninja"
cmake --build target/softfloat-c --target softfloat_demo
```

Clang のパスは導入先に合わせます。出力は `target/softfloat-c/softfloat_demo.wax`。ソースを同梱しているため、ビルド時のダウンロードは不要です。Windows で一時フォルダの書き込みが制限される場合は、作成済みのビルドディレクトリに `TMP` と `TEMP` を設定してください。

`src32_add_library(berkeley_softfloat ...)` にライブラリのソース、ヘッダー検索先、定義をまとめ、`src32_add_executable(... LIBRARIES berkeley_softfloat cpt32_display)` でリンクします。ホスト用 `.a` は利用しません。CMake は `-O2`、回帰テストは `-O0/-O1/-O2` で検証します。Clang 20 が生成する `llvm.usub.sat.i16` は LLVM/SRC32 バックエンドで処理します。

## 実行・検証

GUI で表示する場合:

```powershell
cargo run --release --bin cpt32 -- target/softfloat-c/softfloat_demo.wax
```

BIOS が WAX entry より優先されるため、BIOS なしの設定を使用してください。テストベンチでは `config.yaml` のない空の作業ディレクトリから起動します。

```powershell
cargo build --release --bin src32_testbench
$root = (Get-Location).Path
New-Item -ItemType Directory -Force target/softfloat-c/run | Out-Null
Push-Location target/softfloat-c/run
try {
  & "$root/target/release/src32_testbench.exe" "$root/target/softfloat-c/softfloat_demo.wax" 42 --cpu scalar
  & "$root/target/release/src32_testbench.exe" "$root/target/softfloat-c/softfloat_demo.wax" 42 --cpu pipeline
} finally {
  Pop-Location
}
```

正常時は両方で `PASS: ... => R1=42`。ゲスト自身が演算結果・フラグと PCG 先頭2行の文字 VRAM を検査します。GUI の画素比較ではありません。`--skip-render-checks` は期待 R1 の検査も省くため指定しません。

回帰テスト（`CLANG` と `SRC32_TESTBENCH` で実行ファイルを指定可能）:

```powershell
$env:CLANG = 'C:/Program Files/LLVM/bin/clang.exe'
$env:SRC32_TESTBENCH = "$PWD/target/release/src32_testbench.exe"
python -m unittest discover -s tools/llvm-src32 -p test_toolchain.py -k softfloat -v
```

## ソースの由来と移植設定

指定の [InfiniBrains/softfloat](https://github.com/InfiniBrains/softfloat) は、直下の拡張版に Release 3c 表記が残る一方、`berkeley-sw` サブモジュールに公式 Release 3e を固定しています。このデモはそのサブモジュールのソースを使用します。コミットと同梱範囲は [vendor/softfloat/UPSTREAM.md](vendor/softfloat/UPSTREAM.md)、ライセンスは [COPYING.txt](vendor/softfloat/COPYING.txt) を参照してください。

- binary32 四則演算・int32変換に必要な C ソースだけをリンクします。ヘッダーに宣言されるその他の API はこのサブセットには含まれません。
- `port/platform.h` は SRC32 の big-endian 用です。`LITTLEENDIAN` は **未定義** にします（0 の定義でも上流の `#ifdef` が真になります）。
- `INLINE_LEVEL=0` で上流の汎用 out-of-line primitive を使用し、ホスト固有 intrinsic と TLS は使いません。整数の64-bit演算は既存バックエンドで処理します。
- `8086` specialization は NaN と例外の方針を選びます。x86 命令への依存ではありません。`0/0` の既定 quiet NaN は `0xFFC00000` です。
- `SOFTFLOAT_ROUND_ODD` を有効にします。デモで使う丸めは nearest-even、toward-zero、nearest-ties-away です。フラグは sticky なので、各演算の検査後に明示的にクリアします。
