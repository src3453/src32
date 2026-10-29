# FPGA 向け DMAC/BMC RTL（src32-sv）

- 実装は `src32-sv/soc/src32_bmc.sv`、`src32_dmac.sv`、テストは `src32-sv/sim/tb_dmac.sv`（`python scripts/run_dmac_sim.py`）。詳細と差異一覧は `src32-sv/doc/dmac.md`。
- エミュレータの DMAC は 1 トランザクション（read または write）ごとに BMC を再調停します。`MAX_BURST` は転送に使われていません。RTL もこれに合わせています。
- エミュレータの `CHn_CONTROL.ABORT`（bit 9）は事実上無効です。BUSY 中は CONTROL 書き込み自体が無視され、非 BUSY の abort は何もしないためです。中止は `ABORT` レジスタか `GLOBAL_CONTROL=0` を使います。
- `CHn_CONTROL` の START はバイト 2（bit 15:8）の書き込みで保持し、バイト 3（bit 7:0）の書き込みで発火します。バイト 3 だけの書き込みでも、保持が残っていれば発火します。
- 疑わしいエミュレータ挙動（修正していません）: `Bmc::grant` の単独要求の近道は `CONTROL=0` を無視します。`MAX_BURST` はバイト単位でクランプします。
- 検証: `cargo test --lib -- devices::dmac devices::bmc`（エミュレータ側。`rtl_reference_*` が RTL テストと同じ期待値）と、`src32-sv` での `python scripts/run_dmac_sim.py`。SoC 統合（`soc_top.sv`）と IDC DATA endpoint は未実装です。
