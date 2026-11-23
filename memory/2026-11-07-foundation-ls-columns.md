# Foundation ls の列幅

- コンソールでは列ごとの最長名に合わせて縦順に並べる。PCG は最終セルを書いた時点で折り返すので、複数列の計算では右端を 1 セル空ける。
- 標準出力のハンドルに FND_CON_GET_SIZE を送り、成功時だけ複数列を試す。リダイレクトと -1 は 1 件 1 行。非 ASCII 名は PCG の表示モードとフォントでセル幅が変わるため 1 列にする。
- DIR_NEXT の name_bytes はビッグエンディアンの u16。逆順に読むと、短い ASCII 名でも長さが 256 倍になり、列幅の計算と名前のコピーを誤る。
- python -m unittest discover -s foundation/tests/shell -k test_ls -v で長い名前の列配置、-1、リダイレクトの改行を確認する。
