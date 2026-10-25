# Foundation `/dev/fb0` で BMP を表示する例

`/bin/bmpview`（`foundation/bin/bmpview.c`）は、非圧縮の BMP（8・24・32 bit）を `/dev/fb0` に表示します。キーを押すと終了してコンソールへ戻ります。

- 8 bit の画像は INDEX8 で、BMP のパレットを CLUT に書きます。24・32 bit の画像は RGB888 です。
- 320×240 より大きい画像は中央を切り出し、小さい画像は黒地の中央に置きます（8 bit は index 0 で埋めます）。
- RLE 圧縮、16 bit、4 bit 以下は `bmpview: ...: unsupported` で終わります。

```
python examples/foundation-bmp/make_bmp.py target/fb-demo/root/sample.bmp 24
python foundation/boot/build.py target/fb-demo
python foundation/run.py --root target/fb-demo/root
```

シェルで `bmpview /sample.bmp` を実行します（`--root` の中身はディスクのルートに置かれます）。検証は `python -m unittest discover -s foundation/tests/bmpview -v`（`FND_MM=nommu` でも）。`/dev/fb0` の仕様は [Foundation API §9.1](../../doc/spec_Foundation_API.md#91-カーネル-vdp-ビットマップドライバ-devfb0) です。
