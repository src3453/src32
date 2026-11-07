# Foundation レイトレーシングデモ

- Tang の `/dev/fb0` は INDEX8 のみ。CLUT の 0〜63 は既存の表示と共有するため、デモは64以降に5種類の濃淡を設定する。
- 画面の幅と stride を確認し、2行ずつ書く。最後の close でパレットとコンソールが復元されるため、描画結果を testbench で取得するときは、描画完了後の10秒間に `--dump-framebuffer` を実行する。
- `examples/foundation-raytrace/README.md` の手順で WAX を `--bin-dir` から組み込む。testbench では `--type '30=raytrace\n'` で起動し、画像の鏡面反射と落ち影を確認する。描画完了から10秒以上待ってから `echo status=$?` を入力すると、シェルへ復帰したことも確認できる。
