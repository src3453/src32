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
python tools/fs2/fs2.py serve disk.img [--port 5640] [--read-only]
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

## ホストOSからマウントする（9P2000）

Python標準ライブラリだけで動く9Pサーバーを起動できます。Linuxではカーネルの
9pクライアントでマウントし、通常のファイル操作が使えます。サーバーは一般ユーザーで
起動し、マウント／アンマウントにはroot権限が必要です。

```sh
# ターミナル1（リポジトリのルートで実行）
python3 tools/fs2/fs2.py serve disk.img

# ターミナル2
sudo mkdir -p /mnt/fs2
sudo mount -t 9p -o trans=tcp,port=5640,version=9p2000,access=any,cache=none 127.0.0.1 /mnt/fs2
ls /mnt/fs2
echo hello | sudo tee /mnt/fs2/hello.txt
sudo mkdir /mnt/fs2/work
sudo mv /mnt/fs2/hello.txt /mnt/fs2/renamed.txt
sudo rm /mnt/fs2/renamed.txt
sudo umount /mnt/fs2
# アンマウント後、ターミナル1で Ctrl-C
```

- `--read-only` で読み取り専用公開。クライアント側もmountオプションに `ro` を指定できます。
- 既定は `127.0.0.1:5640`。`--host` と `--port` で変更できます。認証・暗号化はなく、
  接続クライアントはFS2のUID 0として操作します。信頼できないネットワークへ公開しないでください。
- 各更新要求をイメージのコピーに適用し、mount検証に成功してからファイルを置換します。
  失敗した要求は保存しません。成功応答時には保存済みです。電源断時の永続性や、複数要求からなる
  アプリケーション操作全体のatomic性は保証しません。大きなイメージでは要求ごとのコピーと検証に時間がかかります。
- 公開中のイメージをエミュレータや別のFS2コマンドで更新しないでください。
  サーバーはメモリ上のイメージを共有し、複数接続の要求を直列化します。
  隣の `<image>.9p-lock` は同じパスのサーバー二重起動を防ぐ協調ロックです。
  このファイルは終了後も残りますが、ロックは終了時に解放されます。他ツールはこのロックを認識しません。
- 読み書き、ファイル／ディレクトリ作成、削除、truncate、stat、chmod、数値UIDのchown、mtime設定、
  同一ディレクトリ内のrenameに対応します。9P2000ではディレクトリをまたぐrenameを表せないため、
  その移動はコピー＋削除か、アンマウントして既存の `fs2.py mv` を使います。
- FS2のowner/other権限をPOSIXへ変換し、groupにはotherと同じ値を表示します。
  chmodのgroup指定は保存せず、owner/otherを保存します。GIDは0固定、atime・リンク・拡張属性・
  特殊ファイル・open時のORCLOSE/OEXCL/OEXECは未対応です。POSIXの全機能を要求する
  アプリケーションや `cp -a` 等は失敗する場合があります。
- 削除後に残るfidはstaleとして拒否し、同じinode番号で新規作成しても別ファイルを参照しません。
  open中のunlink後も読み書きを継続するPOSIXの挙動は提供しません。
- Windows/macOSは標準の9pマウント機能を持たないため、9P2000対応の外部クライアントが必要です。
  Linux以外のクライアントでのマウント動作は未検証です。

テストは実TCP接続でプロトコルと保存結果を検証します。実際のLinuxカーネルでの
mount検証は別途root権限のあるホストで実施してください。
