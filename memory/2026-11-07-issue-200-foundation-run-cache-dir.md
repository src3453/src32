# Foundation の起動スクリプトから成果物キャッシュを指定する

- `foundation/run.py --cache-dir DIR` は、呼び出し元を基準に絶対パスに変換してから `boot/build.py` に渡す。子プロセスはリポジトリルートで実行する。
- オプションの省略時は、既存の `FND_ARTIFACT_CACHE` を子プロセスへ引き継ぐ。明示したパスの優先順位は `boot/build.py` の引数処理に従う。
- `python foundation/run.py --build-only --cache-dir target/foundation-artifacts` を同じ条件で繰り返すと、エミュレーターを開かずに `Reused` の行を確認できる。
