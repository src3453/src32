# Tang Console Foundation 最小構成

- ユーザー指示で `fpga-foundation` 専用 worktree を使用する。元の `src32/main` は変更しない。この worktree は GitHub Issue なしで作業する（AGENTS.md に明記）。
- `boot/build.py --platform tang-console` で UART 専用 console、CPU コピー、同期 RAM ディスクを選択する。標準 cpt32 ビルドは従来の PCG/IDC/DMAC 経路を使う。
- 既定は SDRAM0 の 16 MiB + 16 MiB 分割。BMC は Main RAM 16 MiB のみを報告する。RAM ディスクは `0x20000000..0x20FFFFFF`、背後の SDRAM0 は `0x01000000..0x01FFFFFF`。後半を Main RAM のアロケータへ含めない。
- `--ramdisk sdram1` は 32+32 MiB の別構成。RTL の `SRC32_SEPARATE_SDRAM=1` と一致させる。今回のボードでは SDRAM1 窓の読み戻しが `007f` の繰り返しとなり未解決。ユーザー承認の分割構成で進めた。
- FPGA console は UART のみ grant する。既存 ANSI/cursor 状態管理を再利用し、PCG/VRAM/input MMIO を発行しない。panic も UART に出す。未実装の画面・音源・乱数サービスを unsupported にし、カーネルから absent MMIO へ触れない。
- RAM コピーは `FND_COPY_DMA_MIN=0`。通常の DMA 有無検出も MMIO read なので、DMA がない FPGA では検出自体を避ける必要がある。
- `blk_io` は既存の user buffer 検証後に mm_copy_* で同期転送する。VM の非連続ページもこの経路で扱う。drive 0 のみ存在する。境界検査は差分で行い lba+count の wrap を避ける。
- 最小 SoC の PeC timer は ch0/ch1 のみ。ch0 IRQ4/100 Hz、ch1 48 MHz accounting。RTC の実時刻はなく起動経過秒を返す。
- UART RX は 16-byte FIFO、console は 100 Hz polling。端末入力は 10 ms/byte で送る。出力中の大量入力のフロー制御は未実装。
- ローダは kernel.bin @0x10000、fndboot.bin @0x200000、foundation.img @0x20000000 をロードする。FPGA 起動は R1/R2 に依存せず staging を固定で参照し boot drive を0にする。manifest の配置・容量を検証し、転送前に BMC 容量も照合する。
- ROM の L 応答 checksum は受信データの和であり SDRAM 読み戻しの証明ではない。R コマンドと --verify で実メモリを確認する。Z はゼロ chunk の転送を省略する。DTR/RTS を上げると FPGA 構成が消えるため low のまま開く。
- Gowin は `../.claude/skills/gowin-build/SKILL.md` に従う。PLL は GowinModGen で生成し、SDRAM clock は337.5度へ変更。分割構成の内部 clk_sys は48 MHz制約に対して49.585 MHz、LUT+ALU12660、FF5242、DSP4。外部SDRAMの確認は実機メモリテスト結果と区別する。
- 実機: split構成で Main RAM 0x1000..0x00FFFFFF と disk全16 MiB が通常/反転2パターンともエラー0。BMC・分離・timer IRQ4/HALT復帰のSoCテストもPASS。最初の4 KiBはテストプログラム自身なので Main RAM の全域テストから除く。
- focused check: `python -m unittest discover -s foundation/tests/console -p test_headless.py -v`（2件）、既存 test_console_host.py（1件）、DiskBootTests.test_image_layout（1件、標準ビルド）が通過。waxホストテストは依存不足でskipされ、成功件数に含めない。
- RTL検証: `scripts/run_timer_sim.py`、`scripts/run_soc_sim.py --phase 337.5`、ローダ unittest。実機の再現用は `scripts/check_sdram.py` / `scripts/board_memtest.py` / `scripts/test_foundation_board.py`。出力は RTL リポジトリの build/ 配下。
- 実機起動とI/O: `Foundation OS (kernel ABI 1.0)` / `Foundation shell 1.0` / `/>` をUARTで確認。uname、free（16384 KiB total / VRAM 0 / PCMRAM 0）、FS2への保存、再読込み、mkdir、cp、コピー先の読込み、lsがPASS。`/fpga-check.txt` と `/fpga-check-dir/copy.txt` に `ramdisk-persist-ok` を残している。ボードはOSのシェルを動かしたままとした。
- 実機ログ: `../src32-sv/build/board-foundation-boot.log`、`board-foundation-check.log`、`board-memtest.log`、`board-split-soc.log`。UARTは115200 8N1、COM7、書込みはFPGA SRAMのみ。
