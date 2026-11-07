# Foundation 用 GUI ランチャーとモジュール選択

- `foundation/launcher.py` は Foundation 専用の Tkinter ランチャー（標準ライブラリのみ）。`tools/cpt32-launcher` は汎用ゲスト用で別物。ロジック（`Catalog`、`validate`、`build_invocation`、`load_state`/`save_state`）は Tk クラスの外に置き、`foundation/tests/launcher` が表示なしで検証する。
- 「オプショナルモジュール」は既存に切り替え手段がなかったため、`boot/build.py` に `--commands` / `--libs`（カンマ区切り、省略=全部、空文字=なし）を追加し、`run.py` が転送する。選べる範囲は `COMMANDS`（/bin）と `LIBS`（lzma, rle, code, crypt, sgc）。シェル・libfnd・サービスは常にビルド。
- 依存は `SGC_COMMANDS`（chip8 → sgc）。build.py はコンパイル前にエラー、ランチャーは選択時に自動で連動させる。
- `OUT/root` は使い回されるので、選ばなかったモジュールの前回出力を build.py が削除する（削除しないと古い .wax がディスクに入る）。
- 保存状態の `known_commands/known_libs` で、保存後に増えたモジュールを有効で始める。
- 環境メモ: コンテナの `python3` は 3.13 で tkinter なし。GUI の煙テストは `apt-get install python3-tk xvfb` 後に `xvfb-run python3.12`。
- 検証: `python3 -m unittest discover -s foundation/tests/launcher`、`discover -s foundation/tests/boot`。実ビルドは `python foundation/boot/build.py OUT --commands ls,cat --libs "" -O0` で bin に ls/cat/shell のみ、lib に fnd のみになることを確認。
