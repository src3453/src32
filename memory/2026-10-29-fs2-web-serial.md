# FS2 Web シリアル更新

- Tang BIOS の I/D/W を利用し、Foundation と同時アクセスしない。全イメージを既存 fs2.py で検査し、変更を準備してから DIRTY を書く。
- セクタごとに読み戻し、全変更完了後に DIRTY を解除する。既存 DIRTY は拒否し、失敗後の修復は行わない。
- 検証: `python -m unittest discover -s tools/fs2-web -p "test_*.py"` と `python -m unittest discover -s tools/fs2`。
