# Foundation IPC ベンチマークの計測契約

- `foundation/tests/perf/bench_ipc.py` は本番カーネルをそのまま `-O1` でリンクし、builtin U-modeの別プロセス間でIPCする。IPCの意味とABIは変更しない。Issue #173。
- `sys_info(2)` のelapsed/kernel/idleは48 MHz、100 Hzのscheduler tickではない。差分はuint32のmoduloで取る。snapshotまでの処理とtimer IRQを含み、snapshot固定費を引かない。結果は各バッチ平均の中央値であり、個別往復のp95ではない。
- 要求/応答は同じ長さ。全バッファのページ先頭整列と+1 byteを比較する。後者は非整列とページ跨ぎが同時に変わる。slotでも約5倍の差があるため、`kmem_copy`の全量byte経路が主な改善候補。ページ処理だけに原因を断定しない。
- LLVM SRC32のstack ABIは8 byte整列までで、`aligned(4096)` なlocal配列はdriverに拒否される。8196 byteのlocal配列の中でポインタを切り上げ、4097 byte以上が残るようにする。builtin code marker間から外部のhelperを呼ばず、globalなユーザーデータも使わない。mailboxは私有64 KiB領域の底、stackは頂点。
- カーネルはユーザーmailboxを直接参照せずmm_read32/mm_write32で扱う。結果を書いたプロセスはexitするとメモリが破棄されるため、完了後sleepしてtick hookが回収する。
- testbenchにUARTはない。計測終了後にカーネルがPCG textへ結果を書き、`--dump-pcg`で回収する。出力途中・欠落・重複・guest失敗はrunnerのエラー。性能に固定の合否閾値は設けない。
- 2026-10-25: CachedでVM/slot × SERVICE/equalを各3起動、PipelineでVM/slotを各1起動、Cached/VMを128反復で1起動。全11ケースが完了。Cached/VMの空IPCはreply_recvで388.05 µs、split比3.21%短縮。基準値・生データは`doc/foundation_ipc_performance.md`と`foundation/tests/perf/baselines/`。
- focused確認: `python foundation/tests/perf/bench_ipc.py target/ipc-bench`。同優先度は別出力で`--priority equal`、Pipelineは`--cpu pipeline --repeats 1`、バッチ固定費は`--mm vm --rounds 128 --repeats 1`で比較。各JSONはGit HEAD/dirty、Clang版、testbench/WAXのhashを保持する。比較時はCPU・最適化・優先度・roundsを一致させる。
