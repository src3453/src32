# pcprof — SRC32 の関数別命令プロファイラ

エミュレータ（パイプライン CPU）がコミットした命令を PC ごとに数え、アセンブリのラベルから関数名に直して集計します。ゲストを改造せず、命令数とコール数だけを見るので、遅い箇所の当たりを付けるのに向きます（実機のキャッシュやバスの待ちは含みません）。

## 仕組み

1. `src32_testbench --profile-pc PATH [--profile-pc-reset-frame N]` が、終了時に `K|U pc 命令数 サイクル数` を 1 行ずつ書きます（`K` = 特権モード、`U` = ユーザーモード）。`--profile-pc-reset-frame` はそのフレームでヒストグラムを捨てるので、起動処理を除けます。パイプライン CPU だけが記録します（`--cpu pipeline`）。
2. `tools/llvm-src32/driver.py --save-temps DIR` が残す `DIR/output.a` を `tools/asm/asm.py` で組み直し、ラベルのアドレスを得ます。ドライバは C の関数名を `llvm_<hex>` に符号化するので、元の名前へ戻し、`__llvm_src32_block_N` などコンパイラ内部のラベルと `.str` などのデータは関数の境界として使いません。
3. 各 PC を直前の関数ラベルに割り当てます。インライン展開された関数は呼び出し元に数えられます。関数の先頭命令の実行回数を「呼び出し回数」とします。

## 使い方

```sh
# 任意のプロファイルを集計（ASM は output.a か、それを含む --save-temps ディレクトリ）
python tools/pcprof/pcprof.py report pc_profile.txt build/temps [--top 25] [--json]
python tools/pcprof/pcprof.py symbols build/temps/output.a --grep sched

# Foundation: シェルコマンドを走らせ、カーネルを関数別に集計する（Clang と release の src32_testbench が必要）
python tools/pcprof/pcprof.py foundation OUTDIR --clang /usr/bin/clang-20 [--command "top -n 4"] \
    [--reset-frame 140] [--frames 700] [--top 25]
```

`foundation` は `foundation/tests/shell` のハーネスでイメージを作り、`--command` を入力し、`OUTDIR/pc_profile.txt` を書いたあと、カーネルを `--save-temps` で作り直して `OUTDIR/temps/output.a` から記号を取ります。作り直したカーネルがプロファイルした実体と違えば警告します。

`foundation` は `foundation/tests/shell/test_shell.py` の `ShellTests`（`setUpClass`、`screen`）と `foundation/boot/build.py` の `driver`、`KERNEL` に依存します。これらを変えたらこのコマンドも直してください。

## 読み方の注意

- 命令数であって時間ではありません。実機では 1 命令あたりのクロックが関数ごとに違い、キャッシュ、MMIO、TLB ミスで変わります。
- `--json` の `cycles` はコミット間のサイクルで、HALT で待っていた時間も次のコミットに入ります。割合の比較には命令数を使ってください。
- 窓（`--reset-frame` 以降）に 1 回だけ走る関数は `instr/call` が大きく見えます（例: プロセス終了時の `vm_destroy`）。

## テスト

```sh
python -m unittest discover -s tools/pcprof -v
```

## 例

`top -n 4`（2026-10-26、パイプライン CPU）でカーネル命令の 24% が `take_next`（IPC 受信ごとに IRQ 16 本を走査）、14% が `proc_user_range_ok`、13% が `kmem_copy` でした。詳細は `memory/2026-10-26-kernel-profile-top.md`。
