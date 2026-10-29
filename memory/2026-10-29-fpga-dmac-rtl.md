# FPGA 向け DMAC/BMC RTL（src32-sv）

- 実装は `src32-sv/soc/src32_bmc.sv`、`src32_dmac.sv`、テストは `src32-sv/sim/tb_dmac.sv`（`python scripts/run_dmac_sim.py`）。詳細と差異一覧は `src32-sv/doc/dmac.md`。
- エミュレータの DMAC は 1 トランザクション（read または write）ごとに BMC を再調停します。`MAX_BURST` は転送に使われていません。RTL もこれに合わせています。
- エミュレータの `CHn_CONTROL.ABORT`（bit 9）は事実上無効です。BUSY 中は CONTROL 書き込み自体が無視され、非 BUSY の abort は何もしないためです。中止は `ABORT` レジスタか `GLOBAL_CONTROL=0` を使います。
- `CHn_CONTROL` の START はバイト 2（bit 15:8）の書き込みで保持し、バイト 3（bit 7:0）の書き込みで発火します。バイト 3 だけの書き込みでも、保持が残っていれば発火します。
- 疑わしいエミュレータ挙動（修正していません）: `Bmc::grant` の単独要求の近道は `CONTROL=0` を無視します。`MAX_BURST` はバイト単位でクランプします。
- 検証: `cargo test --lib -- devices::dmac devices::bmc`（エミュレータ側。`rtl_reference_*` が RTL テストと同じ期待値）と、`src32-sv` での `python scripts/run_dmac_sim.py`。IDC DATA endpoint は FPGA 側で未接続です（SoC 統合は下記）。

## SoC 統合と実機確認（src32-sv、2026-10-29）

- `soc_top.sv` で CPU 側 `cb_*` と DMAC マスターポートをバスで切り替える。DMA 要求がなければ CPU は素通し。BMC grant 後は CPU の開始済みトランザクション（L1 填充は 8 ワード目、`src32_l1.m_last`）を待って所有権を渡す。DMAC が Main RAM へ書くと `x_inv/x_line` で L1 の該当ラインを無効化する（外すと `sw/dma_test.a` が手順 D で失敗する）。
- 初回合成は `clk_sys` 36 MHz で未達だった。原因は、バスデコード → `accept` → レジスタ書き込み、バス ACK → チャネル更新、BMC の調停が 1 サイクルに入っていたこと。BMC/DMAC のレジスタアクセスを 1 段（timer/IDC と同じ）、DMAC シーケンサを IDLE/LOOK/CHECK/BUS/FIN の段に分け、BMC の実効優先度をレジスタ化して解消した。配置オプションは 3 でのみ 48 MHz に達した（0/1/2/4 は未達）。ビルドは 20〜50 分。
- 実機（Tang Console 138K、SRAM 書き込み）で `python scripts/uart_load.py COM7 build/dma_test.bin` が `DMA-PASS`。SDRAM フルテスト（`board_memtest.py`）も合格。
- **未解決（DMA とは無関係）**: コミット済み HEAD（DMA 統合前の `7e1b9d1`）をビルドして書き込んでも、Foundation が起動しない（BIOS は `Starting stage1.` まで進み、UART 経由のカーネルロードもシェルが出ない）。統合前の `build/foundation-fetch` のビットストリームは同じ手順で起動する。作業ツリーの `rtl/src32_core.sv`・`soc/pec_rng.sv` は HEAD と異なる未コミットの状態で、`foundation-fetch` 当時のコアとも違う。原因の切り分け（コア、RNG、BIOS の差）は未了。
- 判断の根拠: DMAC がバスを取らない対照ビルド、HEAD の素のビルドのどちらも同じ症状で、DMA 統合ありの版と区別できない。
