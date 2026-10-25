# Foundation / pkcl のマージ競合解消

- DOOM 対応の `FND_EXEC_IMAGE_MAX` / `FND_SPAWN_IMAGE_MAX`（3 MiB）を維持し、pkcl の fork・pipe・環境変数・BusyBox ash 対応を統合した。取り込み側の 1.5 MiB へ戻さない。
- staging は `0x00100000..0x00400000`。単体の上限とは別に、WAX と依存 WAL と 4-byte alignment padding の合計をカーネルがコピー前に検査する契約を維持する。
- NOMMU BusyBox の 1 MiB 未満の検査はサイズ回帰用。ash 構成の 1.5 MiB 未満の検査も構成固有のサイズ基準として維持し、実行上限と混同しない。
- `doc/spec_pkcl.md` は 3 MiB の上限説明と MMU/ash の設定・自己 exec・argv[0] の説明を両方残す。
- 検証: `python -m unittest discover -s foundation/tests/pkcl -v` はゲスト I/O 1 件成功、外部 BusyBox / Newlib ソース未指定により 4 件スキップ。
- `python -m unittest discover -s foundation/tests/wax -v` はホスト C コンパイラ未検出でスキップ。`CC=C:/Program Files/AMD/ROCm/6.4/bin/clang.exe` を指定しても既存の `-fPIC` が Windows MSVC target で未対応のため実行できなかった。
- 機械的な統合で新たな仕様変更はないため、新規 Issue は作成しない。
