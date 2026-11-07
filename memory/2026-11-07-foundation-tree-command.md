# Foundation の tree コマンド（/bin/tree.wax）

- `foundation/bin/tree.c`。`tree [-d] [path...]`。出力は ASCII の枝（`|-- `、`` `-- ``、継続 `|   `/`    `）。UTF-8 の罫線は PCG フォントに頼れないので使わない。
- 並びは `ls` と同じ（英字のみ大小無視、他は UTF-8 バイト順）。fs の列挙順は不定（API §7）なので各階層を集めてから整列する。
- 再帰は 32 階層まで。深いものは名前だけ出して入らない。階層ごとにヒープへ集め、再帰の前後で prefix 文字列を差し替える（グローバルバッファ）。
- 合計行は全 path 通しで `N directories, M files`（単数は `directory`/`file`、`-d` は directories のみ）。ルートは数えない。エラーは `cmd_report` で出し、残りの path を続け、終了コードは最初のエラー。
- Issue は作っていない（`fpga-foundation` の worktree 方針: AGENTS.md 冒頭）。
- 検証: `python -m unittest discover -s foundation/tests/shell -p test_shell.py -k test_tree`（VM と `FND_MM=nommu`）。この環境では Clang 20 + llvmlite 0.46 を使った（`CLANG=/usr/bin/clang-20`）。
