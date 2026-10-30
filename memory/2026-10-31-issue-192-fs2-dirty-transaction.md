# FS2: 更新操作を1トランザクションとしてDIRTYを戻す (#192)

- 以前は DIRTY=0 に戻すのが `fs2_unmount` だけで、fs サービスに unmount 経路がないため、書込後は常に DIRTY=1 が残った（電源断で終わる Tang では毎回 fsck 待ち）。
- `fs2_write/truncate/create/remove/rename/set_owner` を `*_tx` に分け、公開関数は `end_change()` で包んだ。内部呼び出し（create→dirent_write→file_write）はラップしないので、トランザクションは公開操作単位。
- 成功時は `clear_dirty()`。失敗時は `tx_writes`（DIRTY書込後に書いたsector数）が0なら変更なしとして戻し、1以上なら `suspect` を立ててDIRTYを維持する。`failed`（I/O失敗）も維持。
- 前提: `sec_write` は書込完了まで戻らない（write-through）。キャッシュ/遅延書込を入れるなら DIRTY=0 の前に flush が必要。
- コスト: 更新1回につき superblock を余分に2回書く（set/clear）。大量の小さな書込が遅くなる場合は、サーバが空いた時点でまとめて戻す方式を検討する。
- 検証: `CC=clang python -m unittest discover -s foundation/tests/fs`（28件）。シェルテストは未実行。
