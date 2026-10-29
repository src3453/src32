# bench_cpu 実機起動の切り分け

- 計測は未完了。COM7（3 Mbaud）で元のFlash構成から起動したFoundationは応答するが、元のSDイメージにはbench_cpuがない。
- UART専用の計測用Foundationは `target/foundation-cpu-perf-uart/` にビルドした。bench_cpuは通常のFoundationアプリと同じ `--wax3 --pie --os foundation --page-align -O1` で再ビルドする。元の非PIE成果物は実機で `bad format (-12)`、PIE版はプロセス起動とsys_info(6)呼び出しまで進んだ。
- IPC表示は `instret * 1000u` を64-bitへ拡幅してから乗算し、整数部と小数3桁を分ける。元の32-bit乗算は大きな命令数でoverflowし、元の表示は小数点位置も誤っていた。修正版WAXのビルドとFS2 checkは成功したが、実機での数値出力は未確認。
- 元のFlash構成（JTAG User Code `0x0000456E`）で計測用FoundationをRAM起動するとシェルは動く。ただしbench_cpuの最初のカウンタ読み出しでkernel panic、cause=`80020000` (bus error)、epc=`00010A70`。この構成で性能カウンタ対応を前提にしてはいけない。
- 既存の `src32-sv/build/foundation-perf-x2/.../soc_top.fs` はSRAM Programに成功し、User Code `0x00007FD6` を確認した。BIOSでのチェックサム転送と全データ読み戻しは通ったが、G後のFoundation出力は確認できなかった。S2によるCPUリセット、CACHE_CTRL (`0xFFFF0304`) を0にする診断でも起動は確認できていない。G先にUARTの開始マーカーを出す最小コードを置いた試行でもマーカーなし。停止位置やRTLの原因は未確定。
- 既存の `foundation-perf-split/prj-output` は `build_split.py` が作った `soc_top_split.sv` を使う。Tclにverilog_defineがないことだけでX2と断定しない。
- 新規splitビルドはGowin 1.9.12.04のgw_shがexit 1、空ログで終了した。ツールクラッシュと起動問題の原因は未解決。
- ボタン表記はS1/S2であり、`s0` はRTLの信号名。Console V32001A回路図6・7ページでは **S2 → EX_KEY.1 → BANK5_AB13 → soc_top.s0**。S1はEX_KEY.0 / AA13。ベースボードの再構成リセットは元のFlash構成へ戻るので、SRAMの計測用構成を維持したCPUリセットと混同しない。S2後にはUser Code `0x00007FD6`の維持を確認した。
- 回路図: https://akizukidenshi.com/goodsaffix/Tang_Mega_138K_Console_32001A__Schematics.pdf
- 診断ログと一時スクリプトは隣の `src32-sv/build/foundation-perf-split/bench-cpu-*.log` と `run_bench.py` / `run_after_reset.py`。これらは診断用で、再現性のある完成した計測ランナーではない。最終的にはProgrammer operation 1 (Reprogram)で既存Flash構成へ戻した。
