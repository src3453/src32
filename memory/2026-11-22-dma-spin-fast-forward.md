# DMA 転送中のスピン早送りと DMA 経路の高速化

#206（C の待ちループの早送り）の続き。他のエミュレータを参考にした提案（DMA を必要時にまとめて実行、ページ表、32-bit デバイスアクセス、IRQ 変化時だけの通知）を入れた。仕様は `doc/spec_CPU.md` §1.4、計測は `doc/emulator-performance-plan.md`。

## DMA 中のスピン早送り（厳密）

- `dmac_wait` のように STATUS をポーリングするループを、メモリ転送（IDC エンドポイントなし、1 チャネルだけ busy）の最中も飛ばす。飛ばした周の DMA 操作は `Bus::run_dma_steps` が同じ順番で連続実行する。
- 根拠: busy なメモリチャネルは位相に関係なく毎サイクル要求を出すので、1 周で DMA に渡る操作回数は BMC の状態だけで決まり、データ（カラーキーで書き込みを省くか）に依存しない。だから比較するのは BMC 全体（`Bmc` に `Copy`/`PartialEq`）と DMAC の enable／busy／irq_status で、ビート位相は比べない（`DmacSpinState`）。進んだ操作数は `Dmac::steps`（チャネル別、キー省略も 1 操作）で測る。
- 位相を比べないことにしたのは、最初に位相を比較したら pipeline で全く効かなかったため（奇数サイクルのループと 2 操作のビートで、先頭ごとに Read/Write が入れ替わる）。BMC の周期がずれる場合のために、CPU 状態だけ一致して DMA 状態が一致しないときは古い probe を最大 3 周保持する（`SPIN_DMA_RETRIES`、`dma_may_realign`）。
- 安全区間 `Dmac::spin_reach`: 行ごとに src/dst の範囲がマップ済みの timing-insensitive メモリに収まるかを見て、転送の最後の操作（STATUS 変化と IRQ）と失敗するビートの手前で止める。カラーキーありは 1 ビート最少 1 操作で数える（下限）。
- 観測の禁止: busy なメモリチャネルの REMAIN／ROWS_REMAIN は `peek_u32_be` を返さない（読むと副作用ありに数える）。これがないと `while (REMAIN >= X)` のように比較結果だけがレジスタに残るループを行き過ぎる。テストの R5 で検出できる。
- ハザード: 転送開始（`Dmac::epoch`）以降に CPU がアクセスした 4 KiB ページ（データ、命令 fetch、cached/JIT の世代問い合わせ）を `Bus::hazard_pages` に記録し、飛ばす DMA の書き込み先と重なれば飛ばさない。ループが書き込み先を読む／同じ値を書き戻す／そこにあるコードを実行する場合に順番が変わるのを防ぐ。テストの R7（書き込み先の監視ループ）で検出できる。Pipeline は L1 がインデックスで DMA 書き込みを snoop するので、書き込み先が Main RAM なら飛ばさない。
- 共有 busy マスク（`Dmac::busy_cell`）は「IDC エンドポイントのないチャネル」だけ。IDC 転送も含めると、対象外の IDC 転送で probe 保持・ページ記録・DMA 状態比較が走って HDD の idc_bench が 1 割遅くなり、飛ばせるサイクルも減った。IDC だけが busy なら `DmaMark::Idle` で従来の静穏区間ルールのまま。

## per-cycle 経路

- `Device::read_be`／`write_be`（複数バイト、バイト順の読み書きと同値）。RAM と VDP の VRAM が実装。DMA のメモリビート、CPU の timing-insensitive 32-bit アクセス、同値ストア判定で 1 回の呼び出しにする。RAM の `write_be` は値が同じなら世代を上げない（世代は等値比較だけに使うので、バイト単位と増分が違っても問題ない）。
- `tick_memory_dma_devices`: busy がメモリチャネルだけで IDC がイベントに届かないなら、IDC abort 境界・IDC tick・IRQ 再計算を省いてビートだけ処理する。
- IRQ: DMA のメモリビートは DMAC の IRQ が変わったときだけ `refresh_device_irqs` する（`service_dma_request`）。システムクロック進行でクロック由来の源だけを読む案も実装したが、HDD の idc_bench で速くならなかった（ばらつきの範囲で遅く見えた）ので取りやめた。
- **落とし穴**: IDC→メモリ転送では、最後の「メモリへの書き込み」ビートが `dma_byte_completed` 経由で IDC のコマンドを完了させ IDC の IRQ を立てる。メモリビートだから IRQ 再計算を省く、とすると IRQ を取りこぼす（`idc::tests::identify_read_and_range_error_follow_raw_image_contract` が検出。以前は毎サイクルの完全更新で隠れていた）。IDC エンドポイントのあるチャネルは省略の対象外。
- デバイス探索: 直前のデバイス → その前のデバイス → 64 KiB ページ表（`page_devices`、1 デバイスが丸ごと覆うページだけ登録）→ 線形探索。2 エントリ目は、IDC 転送で「DMA の RAM 書き込み」と「CPU の IDC ポーリング」が交互に来て直前の記憶が毎回外れるため。以前の線形探索は RAM が先頭なので即座に当たっていて、ページ表を引くほうが遅かった（前回ページ表を見送った理由と同じ）。デバイスは追加のみ（削除・並べ替えなし）が前提。
- 性能の落とし穴: cached CPU はループ先頭ごとに `skip_spin` を呼ぶので、そこで DMA 状態（`dma_mark`）を毎回作ると IDC 転送中の HDD ベンチが 1 割遅くなった。CPU 状態が一致したときだけ取り、Idle は `#[inline]` の早期判定で返す。

## 検証

- `cargo test --test dma_spin_fast_forward`（全 CPU 方式、`--features jit` でも）: 1D/2D/カラーキー/ROP/固定 src・負の stride/RAM と VRAM 宛ての転送を、スタック spill 付きのポーリングとタイマー IRQ のもとで早送りあり・なしをロックステップ比較し、REMAIN 待ちと書き込み先の監視ループも含む。REMAIN の扱いかハザード判定を外すと失敗することを確認済み。
- `cargo test --test spin_fast_forward`、`cargo test --lib`、flappy-c の `FLAPPY_PROFILE` 版の描画サイクル数ログが全 CPU 方式で変更前とバイト単位で一致。
- Foundation のシェルテスト（31 件、カーネルの DMA コピーを含む）、flappy-c のテスト 4 件、`test_cpt32_dmac_blit_helpers` も通過。
- 既存の失敗（変更前から）: `cow::tests::rejects_crc_sequence_range_and_base_mismatch`、`--features jit` の `cpu_jit` `fallback_memory_arithmetic_short_mode_and_halt_preserve_cycles`（831/319）。`cpu_bench --platform` は変更前から "pipeline workload did not observe the expected hazards" で動かない。
