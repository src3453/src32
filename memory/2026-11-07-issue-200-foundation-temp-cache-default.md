# run.py の既定キャッシュを OS の一時ディレクトリに置く

- `run.py` の保存先は `--cache-dir`、`FND_ARTIFACT_CACHE`、OS の一時ディレクトリの順で選ぶ。一時ディレクトリではチェックアウトの絶対パスから安定したフォルダ名を作り、再実行でも使い回す。
- `python foundation/run.py --build-only` を保存先未指定で 2 回実行し、2 回目の `Reused` を確認する。
