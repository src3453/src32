# ls の通常表示をタブ区切りにする（#184）

- `foundation/bin/ls.c` の通常表示は、ソート済みの名前の間だけに HT を出し、空でない一覧の末尾に LF を 1 つ出す。空ディレクトリは出力せず、単一ファイルは名前と LF を出す。
- `-l` の各行と集計、複数パスの見出しと空行は維持する。リダイレクト時も HT/LF をそのまま書く。
- console は HT を次の 8 桁境界まで空白で埋める。PCG の画面テストでは `a.txt\tB.txt` は `a.txt   B.txt` になる。ルートの一覧には他の名前も同じ行に続くため、行全体との一致を要求しない。
- `test_ls_tab_output` は画面上の複数パス・空ディレクトリ・単一ファイルの境界と、FS2 上の出力ファイルの実バイト（末尾タブなし、LF、空出力）を検証する。`test_ls_long` は詳細表示を検証する。
- 検証: `python -m unittest discover -s foundation/tests/shell -p test_shell.py -k test_ls -v`。同じコマンドを環境変数 `FND_MM=nommu` でも実行する。Windows では必要に応じて TEMP/TMP を `target/task-tmp` に向ける。
