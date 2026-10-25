# Tang Console UART 3 Mbaud とパイプライン CPU の実機確認

- `soc_top.sv` の `UART_BAUD` 既定を 3_000_000 に変更（DIV=16。`tb_soc.sv` は元から DIV=16 なので RTL シミュレーションは変更なしで PASS）。`uart_load.py`・`test_foundation_board.py`・`board_memtest.py`・`check_sdram.py` の既定 baud も 3 Mbaud。UART は FT2232 の ch.B（COM7）で 12 Mbaud まで可能。
- ロード時間: 115200 + `--verify` は約17分、3 Mbaud・verify なしは約50秒（Z chunk の往復と USB 遅延が支配的）。`--verify` は 3 Mbaud でも読み戻しが遅く 7 MB 地点まで約 400 秒かかり、完走を確認していない。通常は付けない（L 応答のチェックサムで各 chunk は確認される）。
- パイプライン CPU（Extension p）は実機で Foundation を起動し、`test_foundation_board.py` が PASS。メモリ全域テスト・timer IRQ テストは未実行。
- 3 Mbaud 版の配置配線はデフォルトで 46.96 MHz（48 MHz 未達）。`--place-option 3` で 48.005 MHz を満たした。`build/pipeline-3m-p3/` を使う。
- `programmer_cli` が 100% CPU のまま戻らなかったことが 1 回ある。プロセスを kill して再実行すれば書き込める。
- ロード直後（`--wait 3`）にテストを始めると、シェル起動中の入力が欠けて 1 回タイムアウトした。再実行は 2 回とも PASS。
- Foundation 実行中は ROM が消えている。再ロードは `programmer_cli` で書き込み直してから。
