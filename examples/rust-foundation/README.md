# Rust の Foundation コマンドのサンプル

`rsdemo [word...]` は Foundation OS の `/bin` コマンドを Rust で書いた例です。[`fnd`](../../sdk/rust/fnd/) クレートで libfnd（`/lib/fnd.1.wal`）を import し、引数を表示して `alloc`（`Vec`、`String`）でソート・連結し、ファイル `/rsdemo.txt` に書いて読み戻してから消します。`rsdemo --panic` は panic handler の出力（STDERR、終了コード 101）を見せます。

リポジトリルートで:

```sh
python foundation/run.py --build-only
python tools/llvm-src32/cargo_src32.py --manifest-path examples/rust-foundation/Cargo.toml \
    --build-std core,alloc --foundation-wal target/foundation-boot/root/lib/fnd.1.wal -o target/rust-bin/rsdemo.wax
python foundation/run.py --bin-dir target/rust-bin
```

1 行目で `/lib/fnd.1.wal` を作り（`run.py` は `target/foundation-boot` にビルドします）、3 行目で `/bin/rsdemo.wax` を入れたディスクで起動します。シェルで `rsdemo pear apple fig` と打つと:

```
Hello from Rust on Foundation!
3 argument(s)
sorted: apple,fig,pear
file: 14 bytes, apple,fig,pear
```

必要なツールと仕組みは [Rust ガイド](../../sdk/rust/README.md) を参照してください。回帰テストは `foundation/tests/rust` です。
