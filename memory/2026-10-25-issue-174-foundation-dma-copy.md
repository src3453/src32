# Foundation 同期DMAコピーの契約と確認

- `kernel/mm.c` の `kmem_copy` は重ならないMain RAMの256 B以上をDMAC ch1で同期転送する。slotは物理範囲そのまま、VMは既存のmm_vm_copy/from/toが両側ページ境界で分割してから渡す。検証済みユーザーポインタでも仮想アドレスを直接DMACに渡さない。
- kernelは割り込み禁止で動くため、ポーリング中にプロセス終了・フレーム回収は起こらない。これは非同期DMAのページpin設計の代用にはならない。ch1 IRQは無効、P0でCPUポーリングとバスを共有する。コピー終了後にのみ復帰する。
- diskのch0とコピーのch1を予約する。`blk.c`がCHANNEL_ENABLEへ1だけ書くとch1のenableを落とすため、ORに変更。helperも自分のstatus/IRQ bitだけをクリアし、他チャネルを保持する。初期化後もbusyなch1を奪わずCPUへfallbackする。
- DMACが有効でもBMC CONTROL=0ではDMAが進まず無限待ちになるため、BMC調停の有効状態も開始前に検査する。DMAC未搭載/無効、Main RAM外、重なり、小さい片もCPUへfallbackする。
- 相対整列位置が同じならCPUのbyte prefix、32-bit DMA本体、CPUのtail。異なる場合は8-bit DMA。閾値1のテストではprefixより短い長さも発生するため、prefixループはlenも検査する。エラー停止時は元の非重複範囲をCPUで再コピーする。重なりは従来のforward CPU copyを保ち、memmoveの契約を追加しない。
- 比較用に `FND_COPY_DMA_MIN=0`。`bench_ipc.py --dma-min 0` と既定256を同じソースで比較でき、JSONは値を記録する。閾値は物理コピー片に対して判定され、短いページ端ではCPU経路になる。
- Cached/VM/reply_recv/64往復では4096 B整列が1688.25→499.45 µs、先頭+1 Bが8893.89→553.14 µs。slotは1644.24→448.82、8804.25→458.21 µs。各3起動が同値。0/32 BはVM約2%増加するため、短いIPC改善とは区別する。PipelineのDMA経路も両backendで通過。結果は`doc/foundation_ipc_performance.md`と`tests/perf/baselines/2026-10-25-copy-*.json`。
- focused: `python -m unittest discover -s foundation/tests/perf -p test_copy.py -v`。ガードbyte付き全整列、閾値境界、重なり、DMAC/BMC無効、ch0との併走、busy/stale error、非連続VMページを閾値256/1 × Cached/Pipelineで検証。専用testbenchはreleaseを優先する。`test_toolchain`はmoduleとしてimportすること。TestCaseクラスを直接importするとunittest discoverがtoolchain全体まで収集する。
- console/fs: `python -m unittest foundation.tests.console.test_console_guest.ConsoleGuestTests.test_bulk_console_and_disk_io -v` をVM/slotで実行。8193 Bの非整列write/read/EOFと4096 Bのconsole ST_WRITEを検証。既存shell/consoleはVM6件・slot3件が通過。WindowsのTEMP/TMP/TMPDIRはworkspace-localに設定する。
- consoleのper-character描画/ANSI/UTF-8解析や入力10 msポーリング、fs内のユーザー空間copy/metadata処理はDMA対象外。サービス全体の速度は未計測で、IPC echoの改善倍率をそのまま当てはめない。
