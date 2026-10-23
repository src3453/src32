# Foundation /dev/fb0 の例

通常のゲストアプリから、カーネルドライバ `/dev/fb0` を開いて256色CLUTと320×240のINDEX8フレームバッファを書きます。1秒表示してcloseするとコンソールに戻ります。MMIO権限は不要です。

リポジトリルートで実行します（PowerShellでも各行をそのまま実行可能）。

```
python foundation/boot/build.py target/fb-demo
python tools/llvm-src32/driver.py examples/foundation-fb/main.c -I foundation/include -I sdk/cpt32/include -O1 --wax3 --page-align --image-id example.fb --import-wal target/fb-demo/root/lib/fnd.1.wal --origin 0x800000 --stack-top 0xC00000 -o target/fb-demo/root/bin/fbdemo.wax
python foundation/run.py --root target/fb-demo/root
```

起動後、シェルで `fbdemo` を実行します。ClangがPATHにない場合、ビルダはWindowsのROCm配下も探索します。必要なら `--clang PATH` を指定してください。

APIは `<foundation/fb.h>`。通常の `fnd_file_open`、`fnd_write`、`fnd_read`、`fnd_close` と、情報取得・形式変更・シーク・CLUT読書きのhelperを使います。RGB555はbig-endian 2 bytes、RGB888はR,G,Bの3 bytesです。

openは排他で、複製ハンドルは位置と寿命を共有します。最後のcloseまたはプロセス終了時、表示設定・CLUT・重なったPCG文字領域をopen時へ戻します。描画中のコンソール出力はこの復元で失われるため避けてください。`ST_SYNC` は表示完了を待ちません。

検証: `python -m unittest discover -s foundation/tests/fb -v`。`FND_MM=slot` でも実行します。仕様は [Foundation API §9.1](../../doc/spec_Foundation_API.md#91-カーネル-vdp-ビットマップドライバ-devfb0)。
