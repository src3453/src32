# console の UART 入出力複製 (#187、#188)

- console の `write_text` で PCG と PeC UART に同時出力する。LINE エコーも同経路。UART だけ LF を CRLF に展開し、書き込み境界をまたぐ CRLF は二重 CR にしない。消去・座標・色・表示モード変更も ANSI で複製し、起動時にも UART を消去・ホームする。端末のサイズ問い合わせはせず、GET_SIZE/GET_CURSOR は PCG の値。
- UART 受信はキーボードと共通の `accept_key` / `dispatch` を使う。LINE/RAW、EOF、Ctrl+C 待機の処理を二重実装しない。UART は物理 HID 状態を持たないので CON_KEY_STATE には追加しない。
- 分割 UTF-8、CR/LF/CRLF、BS/DEL、Ctrl+C/D、基本 CSI/SS3 を `con_uart.c` でキーに変換する。単独 ESC と未完 CSI は100 Hz の tick で5 tick待つ。修飾付き/未知 CSI は終端まで破棄する。ポーリングは最長1 tick の IPC 待機後とメッセージ間に実施し、UART は256キーまで。満杯の共通キーキューには後着分を追加しない。
- `foundation/boot/build.py` の console grant は5窓になった。UART は `0x80040000..0x80040010`、PeC 入力は従来の `0x80040030..0x80040180`。ディスクイメージの grant 数テストも追従が必要。
- console は UART の設定・割り込みを変更しない。送信は debug_write と同じ DATA_RW 投入で TX_READY を待たない（既存 #138）。FPGA 上の FIFO と転送速度は未検証。console と `/dev/uart0` は同じ受信キューを消費するため、直接 read の併用は入力を取り合う。
- `src32_testbench` は UART を常時接続する。ホスト標準入出力には接続せず、`--uart-rx FRAME=PATH` で生バイトを注入、`--dump-uart PATH` で送信バイトを保存する。PCG ダンプやハーネス出力と混ぜない。以前の「testbench には UART がない」は現状に当てはまらない。
- エミュレータ UART の `print!("{}", data as char)` は0x80以上を再符号化するため、stdout の `write_all(&[data])` に修正 (#188)。`tests/uart_output.rs` は子プロセスの実 stdout を取得し、日本語・NUL・ESC・0xFF のバイト列が保存されることを確認する。
- 検証: `cargo build --release --bin src32_testbench`、`cargo test --test uart_output`（2件）、console 全12件を VM/nommu で実施。追加の UART 専用2件も VM/nommu で実施し、シェルとキーボードの混在、分割 CSI/UTF-8、LINE 編集・短い read・EOF、RAW の非エコーを確認。`python -m unittest discover -s foundation/tests/disk -k image_layout -v` も通過。
- focused コマンド: `python -m unittest discover -s foundation/tests/console -v`、`FND_MM=nommu` を設定して同じコマンド。Windows は作業用 `.tmp` を TEMP/TMP に設定する。Python launcher が install path access denied を返す場合は `C:/Users/y2k34/AppData/Local/Python/pythoncore-3.14-64/python.exe` を直接使う。
