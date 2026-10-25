# Foundation 起動ディスクへの外部 WAX の追加 (#182)

- `foundation/run.py --bin-dir DIR` と `foundation/boot/build.py OUT --bin-dir DIR` は、直下の `.wax`（拡張子の大文字小文字不問）を毎回 `/bin` に追加する。複数指定でき、最後の指定が優先される。WAL・WAD・サブディレクトリは含めず、データや依存ライブラリは既存の `--root` で渡す。
- 外部 WAX をビルド開始前に bytes として保持する。出力先を指定した場合でも、ビルド中の上書き前の WAX をディスクへ取り込める。標準ファイル → `--root` → `--bin-dir` の順で適用し、実行許可は既存の MODE_EXEC（0x003D）を使う。
- FS2 は Unicode case folding で名前を比較する。Python の辞書へ単純に update した後にパスで sort すると、`game.wax` と `GAME.WAX` の優先順位が逆転する。`overlay_files` は FS2 の `fold` で一致する古いパスを取り除いてから追加する。
- 空・不存在の `--bin-dir` はコンパイル開始前にエラー。追加先の標準コマンドも上書きできるため、外部コマンド専用の出力ディレクトリを指定する。
- `run.py --ram-mib N` は既存の YAML 設定範囲 1〜256（既定 16）を使い、`--disk-blocks N` は boot builder へ転送する。DOOM は RAM 64 MiB と WAD を置けるディスク容量も必要で、WAX のコピーだけでは動かない。
- 検証: `python -m unittest discover -s foundation/tests/boot -v`。実 FS2 イメージ上の実行許可・同名優先順位・大文字拡張子・非再帰・ソース snapshot、不正ディレクトリの早期拒否、launcher の引数転送と RAM 設定を確認する。
- 上記 3 テスト通過。`python foundation/boot/build.py target/foundation-bin-dir-check --bin-dir target/foundation-doom --disk-blocks 131072 -j 6` で実ディスクを生成し、取り込まれた DOOM などの WAX が元ファイルと byte 単位で一致すること、mode が 0x003D、FS2 全検査が正常なことを確認した。
