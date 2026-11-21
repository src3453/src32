# DMAC BitBlt（矩形転送・ROP・カラーキー）

- 仕様は `doc/spec_BMC_DMAC.md` §2.4。SGC には BitBlt を入れない方針（`spec_SGC.md`）なので、矩形コピーは DMAC の機能として足した。
- **ID は `0x444D4101` のまま変えない。** Foundation の `kernel/mm.c`（`kmem_copy` の DMA 経路）と BIOS が ID を完全一致で照合しているため、版数を上げると DMA コピーが黙って CPU fallback になる。機能の有無は新設の `CAPS`（`0x018`、bit 0 = BitBlt）で示す。旧 DMAC では未定義オフセットの規則で 0 が読める。
- 拡張レジスタは `0x200 + n*0x20`（ROWS、SRC_STRIDE、DST_STRIDE、KEY、ROWS_REMAIN）。既存の channel ブロック `0x100 + n*0x20` は `0x1C` しか空きがないので別ブロックにした。CHn_CONTROL は bit 6 MODE_2D、bit 7 KEY_EN、bit 11:10 ROP。bit 12 以上は書き込み時に落として 0 を読む（`CONTROL_WRITABLE`）。
- MODE_2D/KEY_EN/ROP のどれかが立つと「BitBlt 転送」で、メモリ専用。開始時に IDC endpoint を指せば code 1、beat がメモリ窓の外（VPU FIFO、拡張カード含む）に出れば code 2。`DmaOp::memory_only` で `Bus::execute_dma_op` が MMIO 例外を外す。IDC/VPU/カード endpoint の既存の例外条件と混ぜないため。
- REMAIN は現在行の残り byte（行が変わると COUNT に戻る）、ROWS_REMAIN は現在行を含む残り行数。総 byte 数は u32 を超えうるので持たない。1D 転送の ROWS_REMAIN は開始時 1。
- ROP≠COPY は 1 beat に src read → dst read（`Phase::ReadDst`）→ write の 3 回のバスアクセス。カラーキーは src の値（ROP 前、WIDTH でマスク）で判定し、一致したら dst read も write もせず `advance_beat` だけ進める。
- FPGA の RTL（`src32-sv/soc/src32_dmac.sv`）にはまだ BitBlt がない。RTL へ移すときは CAPS を 0 のままにするか、同じ挙動を実装してから 1 にする。
- 検証: `cargo test --lib -- devices::dmac devices::bmc`（`blit_*` と `caps_reports_blit_and_id_is_unchanged`）。`cargo test --lib` の `cow::tests::rejects_crc_sequence_range_and_base_mismatch` はこの変更の前から失敗している。
