# FS2 host tool

`fs2.py` は [FS2 仕様](../../doc/spec_FS2.md) のホスト側ツール（§12 の 1）です。raw image をメモリに読み、操作が成功して §10 の mount 検証に通ったときだけファイルへ書き戻します。失敗したコマンドはイメージを変えません。UID 0 として動くので、mode の検査は省きます（§8。構造上の制限は省きません）。

```sh
python tools/fs2/fs2.py format disk.img --blocks 2880 [--boot-blocks 0] [--inodes 256]
python tools/fs2/fs2.py put disk.img hello.wax /bin/hello.wax [--mode 0x3D]
python tools/fs2/fs2.py put disk.img root/ /          # ホストのディレクトリは再帰的にコピー
python tools/fs2/fs2.py mkdir disk.img -p /home/user
python tools/fs2/fs2.py ls disk.img /bin
python tools/fs2/fs2.py get disk.img /bin/hello.wax out.wax   # cat は標準出力へ
python tools/fs2/fs2.py rm disk.img /bin/hello.wax             # 空のディレクトリも消せる
python tools/fs2/fs2.py mv disk.img /a /dir/b
python tools/fs2/fs2.py chmod disk.img 0x3D /bin/tool.wax      # FS2 の mode（POSIX の 0755 ではない）
python tools/fs2/fs2.py chown disk.img 1000 /home/user
python tools/fs2/fs2.py install-boot disk.img fndboot.bin [--stage1 stage1.bin]
python tools/fs2/fs2.py check disk.img [--allow-dirty]
python tools/fs2/fs2.py info disk.img
```

- パスは `/` から始めます。`.` と `..` は成分ごとに解決します（§11.1）。名前の比較は、同梱の `CaseFolding-16.0.0-CF.txt`（Unicode 16.0.0 の C と F）による case folding です。ホストの Python の Unicode 版には依存しません。
- `install-boot` は予約ブート領域に書き、末尾を 0 で埋めて `boot_size` を更新します（§3）。`--stage1` は block 0 のブートコード（510 bytes、または `55 AA` で終わる 512 bytes）も書きます。
- `check` は §10 の mount 検証です。`--allow-dirty` は fsck の検査モードで、DIRTY = 1 を許します。修復（repair）はまだありません。
- 時刻は現在時刻です。環境変数 `SOURCE_DATE_EPOCH` があると、その秒（ms = 0）を使います。
- Python からは `format_image`、`Volume`（`put`、`mkdir`、`remove`、`rename`、`install_boot` など）、`check_image` を使えます。Foundation のディスクイメージは `foundation/boot/build.py` がこれで作ります。

```sh
python -m unittest discover -s tools/fs2 -v
```

`CaseFolding-16.0.0-CF.txt` は Unicode, Inc. のデータで、Unicode License V3 に従います（ファイルの先頭に全文があります）。
