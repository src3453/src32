# Foundation の WAX/WAL 成果物キャッシュ

- `foundation/boot/build.py --cache-dir DIR` または `FND_ARTIFACT_CACHE` で、llvm-src32 ドライバーの WAX/WAL と raw kernel をローカルに保存する。Clang の IR キャッシュは別に設定できる。
- ドライバーの依存ファイルには C 入力とヘッダーに加え、`--exports` と `--import-wal` の入力が含まれる。キャッシュはそれらの内容を照合し、使用中のツールとコンパイル引数もキーに含める。
- `foundation.wax` はドライバー出力を保存してから FNDBOOT ブロックを足す。ディスク用の FNDBOOT と FS2 は毎回組み立てるため、キャッシュディレクトリはディスクに取り込むツリーから外す。
- `python -m unittest discover -s foundation/tests/disk -v` でヘッダー変更による無効化とキャッシュ復元後の BIOS ディスク起動を確認できる。
