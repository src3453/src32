# LauncherからのCPT32PKG保存

- `tools/cpt32-launcher/launcher.py` の `save_cartridge` は現在の有効なディスク・拡張カードとraw ROMをZIPへコピーする。GUIは `_effective_document()` の未保存の編集内容を渡す。無効なスロットは含めない。
- CPT32PKG v1のmanifestは未知キーを拒否する。ホスト用config.yamlをそのままmanifestへ混ぜない。CPU・メモリ容量・BIOS・フォント・音楽・ディスク速度は本形式では復元できず、別途Launcherプロファイルを使う。WAX・sol・既存パッケージをrawプログラムとして同梱しない。
- ZIP内の名前は固定の `program.bin`、`disks/slotN.img`、`plugins/slotN.cpt32card` にする。元のファイル名に含まれる区切り文字や同名衝突を引き継がない。slot/type/read_onlyをmanifestのslot/type/modeへ変換する。
- ROMは16 MiB以下、ディスクとカードは64 MiB以下。ディスクは空でなく512 byte単位。拡張カード内部の検証は既存の起動時ローダーに委ねる。
- 保存ごとに新規UUIDを生成するため、同じ出力名でも別カートリッジのCOW領域を使う。既存のCOW差分を継続する再パッケージ機能ではない。
- 入力への上書き（hard linkを含む）を拒否し、出力先と同じフォルダーの一時ZIPを完成させてから `os.replace` する。失敗時は既存出力を維持して一時ファイルを除去する。
- 保存処理はワーカースレッドで実行し、Tkへのアクセスはイベントキューを通じてメインスレッドで行う。ワーカーへTkオブジェクトを捕捉させない。起動中／保存中の相互実行を禁止する。
- 検証: `python -m unittest discover -s tools/cpt32-launcher -p 'test_*.py' -q` は26件成功。Windows sandboxの既定TEMPはアクセス拒否となるため、実行時だけTEMP/TMPをworkspace内の検証用ディレクトリへ指定した。
- 実際のLauncherウィジェットツリーでボタン表示を確認。Pythonで生成したROM＋slot 3 read-only HDDのパッケージを一時Rust integration testから `Cartridge::load` で読み込み、内容・slot・modeを検証した。`cargo test --test launcher_export_smoke --test cpt32pkg_runtime --quiet` は生成物の1件＋既存9件成功。一時fixtureに依存するsmoke testは検証後に削除した。
