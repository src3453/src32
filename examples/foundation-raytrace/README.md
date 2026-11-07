# Foundation レイトレーシングデモ

Foundation シェルから `raytrace` を実行すると、球と市松模様の床を上から順に描きます。球と床の交差を CPU で計算し、平行光源による影と、中央の球に映り込む床や球を描きます。描画完了から10秒後にシェルに戻ります。

計算は32ビットの固定小数点で、鏡面反射は1回です。160×120のレイをそれぞれ2×2画素に拡大し、320×240の INDEX8 として `/dev/fb0` に書きます。CLUT は64番以降を使用します。MMIO 権限や外部アセットは不要です。

## ビルドと実行

リポジトリルートで、Clang と Python（llvmlite 入り）を使います。最初のビルドでリンクに使う libfnd を生成します。Clang が自動検出されない場合は各コマンドに `--clang PATH` を追加してください。

```powershell
python foundation/boot/build.py target/raytrace-base
python tools/llvm-src32/driver.py examples/foundation-raytrace/main.c -I foundation/include -I sdk/cpt32/include -O2 --wax3 --page-align --image-id example.raytrace --import-wal target/raytrace-base/root/lib/fnd.1.wal --origin 0x800000 --stack-top 0xC00000 -o target/raytrace-app/raytrace.wax
python foundation/run.py --bin-dir target/raytrace-app
```

エミュレータの Foundation シェルで `raytrace` と入力します。

Tang Console の HDMI に表示する場合は、同じ WAX を Tang 用イメージに同梱します。

```powershell
python foundation/boot/build.py target/raytrace-tang --platform tang-console --bin-dir target/raytrace-app
```

既存の Tang Console 起動手順で `target/raytrace-tang/fpga-boot.json` を使用し、UART の Foundation シェルで `raytrace` と入力します。[Tang Console FPGA 用ビルド](../../foundation/README.md#tang-console-fpga-用ビルド)
