# Foundation TinyBASIC 1.0

Foundation の `/bin/tinybasic.wax` は、既存の `examples/tinybasic-c/main.c` を共用する C 製インタプリタです。
WAX v3 として libfnd 1.1 を import し、Foundation のシェルから起動します。

## 起動

```text
tinybasic                 # 対話 REPL
tinybasic program.bas     # cwd を基準にファイルを読み込み、RUN して終了
tinybasic /dd/0/demo.bas  # ファイルシステム上の絶対パスも使える
```

引数はプログラムのパスを1個まで受け付けます。拡張子による形式判定はせず、行番号付き BASIC テキストとして読みます。
ファイル指定時は画面を消さず、ヘルプ・OK・LOADED を表示しません。プログラムの末尾、END/STOP、BYE/QUIT で終了し、シェルへ戻ります。
正常終了は0、読込失敗は Foundation のエラーコード、構文・実行エラーは `FND_E_BAD_ARG` (-2) です。
エラーは既存の `? ERROR IN LINE n` で行番号を表示します。

引数なしでは、起動時にコンソールを消去してヘルプを表示し、`OK` / `> ` を出します。
行番号を付けるとプログラムを保存・置換し、行番号だけなら削除します。`RUN` で変数をクリアして先頭から実行、`LIST` で一覧表示、`NEW` でプログラムを消去します。
`BYE` / `QUIT` と入力先頭の EOF（コンソールでは Ctrl+D）でプロセスを終了します。

```text
10 FOR I=1 TO 3
20 PRINT I
30 NEXT I
SAVE "demo.bas"
NEW
LOAD "demo.bas"
RUN
BYE
```

シェルから `tinybasic demo.bas` を実行すると、この例では1、2、3を表示して終了します。

## 言語とファイル

- 既存 C 版の整数式、A-Z、`@(n)`、LET、PRINT（`#N` 幅指定）、INPUT、IF/THEN、GOTO、GOSUB/RETURN、FOR/NEXT、REM、TRON/TROFF を使えます。
- 32-bit 整数、行番号1〜9999、入力は1行119 bytesまで、配列1000要素、実行ディレクトリ1024行、GOSUB 64段、FOR 32段です。
- 既存版同様、FOR の本体は NEXT で判定する前に1回実行します。
- `LOAD "path"` / `SAVE "path"` は fs のパスを扱います。cwd、権限、大小文字の扱いは Foundation の fs に従います。
- LOAD は成功時に現在のプログラムを置き換え、実行中なら停止します。読込・サイズエラーでは現在のプログラムを保持します。
- SAVE は行番号順の CRLF テキストを作成または置換します。sector のパディングはありません。読込は LF/CRLF の行番号付きテキストに対応します。
- ファイル用バッファは128 KiBです。LOAD は超過を検出して拒否し、SAVE はシリアライズが収まることを確認してからファイルを開きます。書込エラー時には途中までのファイルが残る場合があります。
- LOAD/SAVE の LBA・sector 指定は Foundation 版では使いません。
- RND はプロセス内の疑似乱数です（初期seed=1）。PeC RNG を直接操作しません。

## 入出力・保護

STDIN/STDOUT を継承し、`tinybasic demo.bas < answers.txt > result.txt` や、REPL 命令列を `tinybasic < commands.txt` として渡す方法に対応します。
INPUT も同じ STDIN を読みます。コンソールでは LINE モードの行編集とエコーを利用し、長すぎる入力は改行まで捨てて `LINE TOO LONG` を表示します。
CLS は CONSOLE の役割があれば画面を消します。実行中の無限ループを Ctrl+C で中断する処理はありません。

プログラム行・AST・文字列・制御スタックは `fnd_mem_alloc` で確保した約1.9 MiB の私有領域に置きます。
固定物理アドレス、PCG/PeC/IDC/DMAC MMIO、独自 IRQ ハンドラや HALT は使用しません。
仮想記憶と `FND_MM=slot` のどちらでも、標準コマンドのスロット5で動きます。

## ビルドと検証

```sh
python foundation/boot/build.py target/foundation-boot
python foundation/run.py --root examples/tinybasic-c/foundation-root
python -m unittest discover -s foundation/tests/tinybasic -v
# 同じテストを環境変数 FND_MM=slot でも実行する
```

起動ディスクには `/bin/tinybasic.wax` が自動的に入ります。
`run.py --root DIR`（または `boot/build.py OUT --root DIR`）はホストの DIR 配下をディスクの `/` にコピーします。
上記の実行例では `/demo.bas` も入り、シェルから `tinybasic demo.bas` で実行できます。自分の `.bas` も同じ方法で配置します。
テストはシェルから実際に起動し、PCG出力、計算・制御フロー、INPUT、ファイル引数、終了ステータス、再起動時の分離、リダイレクト、保存されたFS2ファイルの内容を確認します。
