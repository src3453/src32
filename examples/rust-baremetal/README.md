# Rust ベアメタルのサンプル

BIOS なしで直接起動する `no_std` の Rust プログラムです。[`cpt32`](../../sdk/rust/cpt32/) クレートで PCG 文字画面に書き、`core::fmt`、スライスのソート、イテレータ、64-bit 整数、浮動小数点（SoftFloat）、トレイトオブジェクトを確かめて、すべて通れば `main` が 42 を返します。

リポジトリルートで:

```sh
python tools/llvm-src32/cargo_src32.py --manifest-path examples/rust-baremetal/Cargo.toml -o rust-baremetal.wax
```

BIOS を設定していないディレクトリから実行します。

```sh
/path/to/src32_testbench /absolute/path/rust-baremetal.wax 42 --dump-pcg
```

画面には `Hello from Rust on CPT32!` と `checks: ok` が出ます。必要なツールと仕組みは [Rust ガイド](../../sdk/rust/README.md) を参照してください。回帰テストは `tools/llvm-src32/test_rust.py` です。
