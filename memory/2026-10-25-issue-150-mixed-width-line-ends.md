# 8×16コンソールの半角・全角混在文と行末

- `con_pcg.c` の `put_wide` は、残り1セルなら最終列を空白にして改行した後、UTF-16の上位／下位byteを同じ行へ書く。残り2セルなら行末へ収め、直後に改行する。今回の検証では実装変更は不要だった。
- `foundation/tests/console/host.c` に、混在文 `a日本bあz` を80通りの開始列から出す検査を追加。通常時と90改行後の最下行（リングが2周した状態）の計160ケースで、カーソル位置と全画面のWIDE_KIND対（01→10）が行内で完結することを確認する。
- `test_mixed_width_line_ends_in_8x16` は、`cat` → libfnd → IPC → console の経路で、混在文の後に残り0・1・2セルとなる境界を検査する。文字面の上位／下位byte、両セルのWIDE_KIND、前景色・背景色を実際のVRAMで確認する。後続のシェルプロンプトが検査箇所を上書きしない位置へカーソルを移動している。
- 検証: `cargo build --release --bin src32_testbench`、`python -m unittest foundation.tests.console.test_console_host foundation.tests.console.test_console_guest.ConsoleGuestTests.test_mixed_width_line_ends_in_8x16 -v`。ゲストの同じテストは `FND_MM=slot` でも成功。
- Windowsサンドボックスの既定一時ディレクトリではアクセス拒否が出たため、`TEMP`／`TMP` をリポジトリ内の `target/console-edge-tmp` に設定して実行した。
- 対象は画面上の2セル文字の行末処理。入力UTF-8をST_WRITE間で分割する挙動は検証対象外であり、API仕様どおり1文字を1回のST_WRITE内へ収める契約を保つ。
