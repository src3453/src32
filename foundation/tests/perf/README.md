# Foundation IPC ベンチマーク

Issue: [#173](https://github.com/src3453/src32/issues/173)

リポジトリのルートから実行します。Clang、Python（LLVM SRC32 driver の依存を含む）、`src32_testbench` が必要です。

```sh
cargo build --release --bin src32_testbench
python foundation/tests/perf/bench_ipc.py target/ipc-bench --mm vm slot
python foundation/tests/perf/bench_ipc.py target/ipc-bench-equal --priority equal
python foundation/tests/perf/bench_ipc.py target/ipc-bench-pipeline --cpu pipeline --repeats 1
python foundation/tests/perf/bench_ipc.py target/ipc-cpu --dma-min 0
```

Tang Console FPGA で Foundation の通常の U-mode プロセスを測るには、
`bench_cpu.wax` を `/bin` に入れた Tang 用 Foundation image をビルドし、
カウンタ対応SoCをSRAMに書き込んでから Foundation を起動します。
このプログラムは ALU、依存ロード、メモリ更新の各 workload を実行し、
各区間で `sys_info(6)` の8カウンタを前後に読みます。値は Foundation
カーネルとシステムサービスを含む SoC 全体の差分です。
出力の `IPC_MILLI` は `INSTRET * 1000 / CYCLE` を整数で計算し、IPCを小数第3位まで表示します。

```powershell
python tools/llvm-src32/driver.py --clang 'C:/Program Files/LLVM/bin/clang.exe' `
    --wax3 --pie --os foundation --page-align -O1 `
    -I sdk/cpt32/include -I foundation/include `
    foundation/tests/perf/bench_cpu.c -o build/bench-cpu/bench_cpu.wax
python foundation/boot/build.py target/foundation-cpu-perf `
    --platform tang-console --root-drive ramdisk `
    --clang 'C:/Program Files/LLVM/bin/clang.exe' --bin-dir build/bench-cpu
python ../src32-sv/scripts/build_fpga.py --layout split --out ../src32-sv/build/foundation-perf
```

Program the generated SRAM bitstream `src32-sv/build/foundation-perf/prj/soc_top/impl/pnr/soc_top.fs`,
then load the Foundation manifest and run the program from the UART shell:

```powershell
python src32-sv/scripts/uart_load.py COM7 --foundation target/foundation-cpu-perf/fpga-boot.json --wait 60
python src32-sv/scripts/fnd_cmd.py COM7 bench_cpu
```

The kernel API returns counter deltas as low/high words; reported IPC is
`INSTRET / CYCLE`. Counter reads and their syscalls are inside the elapsed
window, along with timer IRQs and any other runnable Foundation services.
The benchmark uses 200,000 loop iterations per workload; use identical
iterations and Foundation/SoC builds when comparing runs. Build and
measurement outputs are retained under `build/` and `target/`.

`--clang` / `CLANG`、`--testbench` / `SRC32_TESTBENCH` で実行ファイルを指定できます。`--rounds` はバッチ内の反復数（既定64、最大512）、`--repeats` は独立した起動回数（既定3）です。出力ディレクトリは条件ごとに分けてください。毎回ビルドし、イメージ、ビルドログ、実行ログ、`results.json` を保存します。`FND_MM` に関係なく `--mm` がバックエンドを選び、ゲストでも実際の選択を検証します。

カーネルのDMAコピー追加後は `--dma-min 256` が既定です。`--dma-min 0` で同じソースからCPU専用イメージを作れます。値は物理的に連続するコピー片の長さに対する閾値で、VMではメッセージ全体が大きくても短いページ端の片はCPUコピーになります。JSONの `copy_dma_min` がない従来の基準値はDMA導入前です。DMA導入の結果は [分析の追記](../../../doc/foundation_ipc_performance.md#同期dmaコピーの試作174) を参照してください。

## 測定対象と単位

- 本番のカーネルを `-O1` でリンクした専用イメージで、独立した U モードのクライアントとサーバーを動かします。IPC や scheduler に計測コードを挿入しません。初期ハンドルの用意だけはカーネル側で行います。
- `time_get(TICK_HZ)` の syscall 対照と、`call → recv → reply → call復帰` の往復を測ります。サーバーの応答後の受信は `reply` + `recv` と `reply_recv` を比較します。要求・応答のデータ量は同じです。
- データは0、32、256、4096 bytes。4096 bytes はページ先頭の整列ケースと、送受信の全バッファを先頭+1 byteにずらしたケースがあります。後者は非整列とページ境界越えの両方を含みます。整列とページ境界それぞれの寄与は、この比較だけでは分離できません。
- 既定ではサーバーが SERVICE、クライアントが APP。`--priority equal` では両方APPです。他のアプリケーション、ディスク、console、WAL は動かしません。コードは builtin 用の共有RX領域、データはそれぞれの私有領域です。
- 各ケースで8回ウォームアップ後、`sys_info(2)` の48 MHzカウンタでバッチを挟みます。`elapsed/kernel/idle` の符号なし32-bit差分を取り、周波数と会計の整合性も検証します。**1 tick = 1/48 µs**で、scheduler の100 Hz tickとは異なります。ホストの実行時間や retired instruction 数ではありません。
- 表示値は**各バッチ平均の中央値**です。`results.json` に各起動の生の差分、平均の最小・最大、往復/秒、kernel/idle を保存します。個々の往復のp95/p99を表す値ではありません。
- 区間には通常のtimer IRQ、ゲストのループ、応答のsequence/長さ確認、終点のsnapshotまでの処理が含まれます。snapshotコストを差し引きません。`time_get` はIPC遅延から引くための補正値ではなく、syscallコストの対照です。128反復でも測り、バッチサイズへの依存を確認できます。
- 各バッチの最終応答のペイロード全体の確認と画面への出力は計測後に行います。要求ごとのsequence、受信長、badge、reply capacityも検証し、故障・不正データ・欠落/重複した結果・ゲスト未完了をエラーにします。性能値そのものに合否閾値は設けません。

CPUモデル、コンパイラ、最適化、反復数、優先度が同じ条件で比較してください。JSONには開始時のGit HEAD、作業ツリーのdirty状態、コンパイラ版、testbenchとWAXのSHA-256を記録します。生成物の同一性はGit HEADだけで判断しないでください。

## 2026-10-25 の測定と改善候補

測定結果と分析は [doc/foundation_ipc_performance.md](../../../doc/foundation_ipc_performance.md)、生データは [baselines/](baselines/) にあります。カーネルの意味を変更せず、まず非整列コピーと短いメッセージの固定費を次の改善対象として切り分けます。
