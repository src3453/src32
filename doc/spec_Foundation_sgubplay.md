# Foundation SGUBプレイヤー

`/bin/sgubplay.wax` はFoundationの標準コマンドです。

```text
sgubplay FILE
```

FS2上のファイルを読み、SGU 2基と共有PCMRAMへ再生します。相対パスは
シェルの作業ディレクトリ基準です。再生終了後はシェルへ戻ります。
ファイルのrepeatフラグが立っている場合は先頭から繰り返します。
Escまたは `q` / `Q` で停止できます。Ctrl+Cによるシェルからの強制終了でも、
カーネルが音源ハンドルを閉じて消音します。

## 曲ファイルをディスクに入れる

`--root` はホストのフォルダーの中身を起動ディスクのルートへコピーします。
例えばPowerShellでは次のようにします。

```powershell
New-Item -ItemType Directory -Force target/music-root/music
Copy-Item path/to/song.sgub target/music-root/music/song.sgub
python foundation/run.py --root target/music-root
```

起動したFoundationのシェルで実行します。

```text
sgubplay /music/song.sgub
```

ホストCLIの `--music-stream` は使いません。曲はFoundation自身がファイルAPIで読みます。
`boot/build.py OUT --root DIR` でビルドだけを行う場合も、生成した
`OUT/foundation.img` をIDC slot 0に接続して起動してください。

## 対応と制限

- [SGUB v1](spec_SGUB.md)、[v2](spec_SGUB_v2.md)、および
  [v3](spec_SGUB_v3.md)のSGU/PCM target 0〜2に対応します。
  WRITEの順序と重複、FILL、COPY_POOLを維持します。
- S3HSなどの拡張カードtargetは未対応です。ファイルの途中に含まれる場合も、
  初期化を書き込む前に `unsupported` で拒否します。
- ファイル全体をメモリへ読み、全ヘッダー・pool・初期化・フレーム列を検証してから
  `/dev/sgu` を開きます。フォーマットの上限は14 MiBですが、実際に読める大きさは
  利用可能なプロセスheapと物理メモリに依存します。特にslot方式のコマンド領域は
  4 MiBで、コード・スタックを含むため14 MiBの曲を格納できません。
- 60 Hzのフレーム進行はOSのtickを累積して求めます。100 Hzの時計では10 ms単位に
  量子化され、VSyncとの位相同期はありません。CPUサイクル数や独自IRQは使いません。
  遅延した場合は操作を省略せず順番に追いつきます。1秒を超える停止時間は1秒に制限して
  再開し、大量の追いつき処理による操作不能を避けます。重い曲は再生が遅れることがあります。
- 音源は排他利用です。他のプログラムが `/dev/sgu` を開いていると `busy` を返します。
  通常終了・停止・出力エラーではハンドルを閉じ、16チャンネルを消音します。
- 成功・手動停止は終了コード0、引数・ファイル・出力エラーは負のFoundationエラーを返します。

## 実装と検証

`foundation/bin/sgubplay.c` がファイル・操作・時計を扱い、
`sdk/cpt32/src/sgub.c` の共通パーサーを `sgub_init_output` で利用します。
出力callbackが `/dev/sgu` のseek/writeへ変換し、IPC上限で分割して転送します。
ユーザープログラムへのMMIO grantやカーネルの機能追加はありません。
初期化列はrepeat時に再実行しません。非repeatの最終フレームを適用した次の
フレーム時刻で消音します。

```sh
python -m unittest discover -s foundation/tests/sgub -v
```

ホスト試験にはCコンパイラ（`HOST_CC`で指定可能）、Foundation実行試験には
Clangと `src32_testbench` が必要です。同じ試験を `FND_MM=slot` でも実行します。
試験はv1/v2/v3、実Furnaceのpool付き出力、完了・繰り返し・停止・再オープン、
不正ファイルと未対応targetの事前拒否、callbackの出力エラーを確認します。
