# sol から libfnd を使う

`main.sol` と `child.sol` は Foundation の U-mode プログラムです。
実際の C libfnd を WAL `fnd` 1.0 として import し、IPC 経由でサービスを使います。
main は slot 5、子は slot 6 に配置します。同じ固定アドレスの slot のプログラムを
親子として同時には実行できません。

リポジトリのルートで実行します（Python、llvmlite、Clang が必要）。

```sh
python examples/foundation-sol/build.py target/foundation-sol
```

生成した `target/foundation-sol/foundation.wax` をエミュレータで開き、
Foundation シェルで `sol-demo` を実行します。`bios.image` を指定している場合は、
Foundation 用の別の作業ディレクトリと空の `config.yaml` から起動してください。
PCG の描画には既存の Foundation 起動設定と同じ `pcg.font` が必要です。
`foundation.img` も生成されます。

テストベンチでの再現手順（絶対パスに置き換えてください）：

```sh
python examples/foundation-sol/build.py target/foundation-sol --stop-ticks 6000
# 空の config.yaml がある別の作業ディレクトリで実行
<repo>/target/release/src32_testbench <repo>/target/foundation-sol/foundation.wax 42 --cpu pipeline --frames 650 --dump-pcg --type '40=sol-demo\n'
```

成功時は `read/close: WAX magic OK`、`hello from sol child`、`from-parent`、
`child: exit 17 (wait closed handle)`、`stream write OK`、`sample: OK` と表示します。
失敗した open の出力引数が保持されること、コンソールのサイズ、stat の配置も確認します。
`fnd_proc_wait` は成功時に PROCESS ハンドルを閉じるので、重ねて close しません。

暫定 bootfs は読み取り専用で、列挙も未実装です。サンプルはそれぞれ
`write-open: read-only` と `directory: not supported` を確認します。
書き込み可能なファイルへの write と列挙成功は KAN-15 の fs 実装後の検証として残ります。
`fnd_write` の成功はコンソールの stream で確認しています。

API とビルダーの契約は [sol/foundation/README.md](../../sol/foundation/README.md) を参照。

```sh
python -m pytest tools/solc/test_sol_libfnd.py -q
```

このテストは実カーネルと WAL をビルドし、Scalar と Pipeline で各3回連続起動します。
コンパイラだけのテストを除き、Clang と `src32_testbench` が必要です。
