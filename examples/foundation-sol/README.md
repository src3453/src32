# sol から libfnd を使う

`main.sol` と `child.sol` は Foundation の U-mode プログラムです。
実際の C libfnd を WAL `fnd` 1.1 として import し、IPC 経由でサービスを使います。
main は slot 5、子は slot 6 に配置します。同じ固定アドレスの slot のプログラムを
親子として同時には実行できません。

リポジトリのルートで実行します（Python、llvmlite、Clang が必要）。

```sh
python examples/foundation-sol/build.py target/foundation-sol
```

生成した `target/foundation-sol/foundation.wax` をエミュレータで開き、
Foundation シェルで `sol-demo` を実行します。**カレントディレクトリは生成された
`target/foundation-sol/run` にします**。その `config.yaml` はPCGフォントと、
fsサービスがルートとしてマウントする書き込み可能なIDC slot 0のディスクを設定します。
`foundation.img` は初期イメージ、`run/disk.img` は実行用のコピーです。

テストベンチでの再現手順（絶対パスに置き換えてください）：

```sh
python examples/foundation-sol/build.py target/foundation-sol --stop-ticks 40000
cd target/foundation-sol/run
<repo>/target/release/src32_testbench <repo>/target/foundation-sol/foundation.wax 42 --cpu pipeline --frames 4200 --dump-pcg --type '40=sol-demo\n'
```

成功時は `write/read/close: 5001 bytes OK`、`directory: 65 entries, UTF-8, rewind, END OK`、
`cleanup: file and directory removed`、`hello from sol child`、`from-parent`、
`child: exit 17 (wait closed handle)`、`stream write OK`、`sample: OK` と表示します。
失敗した open の出力引数が保持されること、コンソールのサイズ、stat の配置も確認します。
`fnd_proc_wait` は成功時に PROCESS ハンドルを閉じるので、重ねて close しません。

サンプルはFS2上に `/sol-result.bin` を作り、5001 bytesを書いてsync/close後に再読込します。
読み取り専用で開いたハンドルへのwriteは `FND_E_ACCESS` と0 bytesになります。
`/data/sol-fixtures` の64個のASCII名と `日本語.txt` は4096-byte応答に収まらないため、
複数応答のキャッシュを検査できます。順序を仮定せず、重複・欠落・UTF-8名と長さを確認します。
途中と終了後のrewind、空ディレクトリ、ENDでの出力保持、4回の途中closeでのヒープ末尾の不変も検査します。
成功時に作成したファイルと空ディレクトリを削除するので、同じ起動中に繰り返し実行できます。

testbenchの停止はfsのunmountではないため、実行用ディスクはDIRTYのままです。
次の起動には初期 `foundation.img` を `run/disk.img` にコピーするか、ビルドし直してください。
テストはCPUごとに新しいコピーを使い、停止後はDIRTYをクリアせずホストで整合性と後始末を検査します。

API とビルダーの契約は [sol/foundation/README.md](../../sol/foundation/README.md) を参照。

```sh
python -m pytest tools/solc/test_sol_libfnd.py -q
```

このテストは実カーネルと WAL をビルドし、Scalar と Pipeline で各3回連続起動します。
コンパイラだけのテストを除き、Clang と `src32_testbench` が必要です。
