# C++17 + LLVM 開発サンプル

Clang で C++17 → LLVM IR → SRC32 ASM → WAX と変換し、CPT32 の RAM 上で実行する最小の複数ファイルプロジェクトです。
`main.cpp` が `counter.cpp` のクラスを使って `7 + 7 * 5 = 42` を計算し、既存の C SDK で PCG 画面に挨拶と結果を表示します。正常終了すると R1=42 を残して HALT します。

## 準備

リポジトリルートから実行してください。Python 3.11 以降、Clang 20、CMake 3.20 以降が必要です。以下は PowerShell の例です。

```powershell
python -m pip install -r tools/llvm-src32/requirements.txt ninja
cargo build --bin src32_testbench
```

## CMake でビルド

```powershell
$python = python -c 'import sys; print(sys.executable)'
$ninja = python -c 'import pathlib, ninja; print(pathlib.Path(ninja.BIN_DIR) / "ninja.exe")'
cmake -S examples/hello-cpp -B target/hello-cpp -G Ninja `
  "-DSRC32_PYTHON_EXECUTABLE=$python" `
  '-DSRC32_CLANG=C:/Program Files/LLVM/bin/clang.exe' `
  "-DCMAKE_MAKE_PROGRAM=$ninja"
cmake --build target/hello-cpp --target hello_cpp
```

Clang のパスは実際の導入先に合わせます。生成物は `target/hello-cpp/hello_cpp.wax` です。
`project(... LANGUAGES NONE)` と `src32_add_executable` を使うため、ホストの C++ リンカや標準ライブラリの検出は不要です。C++ の `.cpp/.cc/.cxx` は C++17、SDK の `.c` は C11 としてコンパイルします。

## CLI でビルド

CMake を使わず、必要な SDK モジュールだけを直接リンクすることもできます。

```powershell
New-Item -ItemType Directory -Force target/hello-cpp | Out-Null
python tools/llvm-src32/driver.py `
  examples/hello-cpp/main.cpp examples/hello-cpp/counter.cpp `
  sdk/cpt32/src/pcg.c -I sdk/cpt32/include `
  --clang 'C:/Program Files/LLVM/bin/clang.exe' -O1 `
  --save-temps target/hello-cpp/ir -o target/hello-cpp/hello_cpp.wax
```

`ir` に翻訳単位ごとの LLVM IR、`linked.ll`、SRC32 の `output.a` が残ります。

## 実行と検証

GUI で文字表示を見る場合:

```powershell
cargo run --release --bin cpt32 -- target/hello-cpp/hello_cpp.wax
```

BIOS の設定がある場合は RAM の WAX entry より BIOS が優先されるため、BIOS なしの設定を使用してください。
テストベンチは空の作業ディレクトリから実行し、ルートの `config.yaml` を読み込ませません。

```powershell
$root = (Get-Location).Path
New-Item -ItemType Directory -Force target/hello-cpp/run | Out-Null
Push-Location target/hello-cpp/run
try {
  & "$root/target/debug/src32_testbench.exe" "$root/target/hello-cpp/hello_cpp.wax" 42 --cpu scalar
  & "$root/target/debug/src32_testbench.exe" "$root/target/hello-cpp/hello_cpp.wax" 42 --cpu pipeline
} finally {
  Pop-Location
}
```

正常時は `PASS: ... => R1=42`。クラスの計算に加え、ゲスト自身が PCG の先頭行と次行の文字 VRAM を検査します。GUI の画素を自動比較するテストではありません。
Linux 等では `ninja.exe` を `ninja` に、テストベンチを拡張子なしのパスに置き換え、シェルの構文も合わせてください。

## C++ の利用範囲

- サンプルは namespace、constexpr、static_assert、クラス、コンストラクタ、メンバ関数、固定幅整数とキャストを使用します。メソッドは `.cpp` に定義し、複数翻訳単位でリンクします。
- 起動関数は `extern "C" int main()`。freestanding Clang では `main` にも C++ の名前修飾が付くため、C linkage を明示します。C SDK のヘッダーもサンプルのように `extern "C" { ... }` で囲みます。
- 例外・RTTI・スレッドセーフなローカル static 初期化は無効です。標準 C++ ヘッダー／STL、new/delete の実装は付属しません。サンプルは `<stdint.h>` を使い、C++ オブジェクトを自動変数として生成します。
- 動的なグローバル初期化・終了時デストラクタ、仮想関数やテンプレート／inline メソッドなどが生成する weak/COMDAT は未対応です。未対応 IR はエラーになります。すべての C++17 機能を利用できるわけではありません。
- 浮動小数点命令など、バックエンドの制限は [ツールチェーン README](../../tools/llvm-src32/README.md) と共通です。

回帰テストは `tools/llvm-src32/test_toolchain.py` にあり、各 C++ 拡張子、全最適化レベル、Scalar/Pipeline、動的グローバル初期化の拒否を確認します。
