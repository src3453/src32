# Foundation 外部ライブラリ作成デモ: liblzma.wal

Igor Pavlov の [public domain LZMA SDK](https://www.7-zip.org/sdk.html) 26.05 を、
Foundation の位置独立 WAL にする例です。SDK の圧縮・展開処理そのものをゲスト CPU で実行します。
SDK は `foundation/lib/lzma/vendor/` に必要な部分だけ同梱しています。取得元と SHA-256 は
同ディレクトリの `UPSTREAM.md`、許諾原文は `lzma-sdk.txt` を参照してください。

## ビルドと利用

Python、llvmlite、Clang が必要です。リポジトリのルートで実行します。

```sh
python examples/lzma-wal/build.py target/lzma-wal --clang /path/to/clang
```

Windows の例:

```powershell
New-Item -ItemType Directory -Force target/lzma-temp | Out-Null
$env:TEMP = "$PWD\target\lzma-temp"
$env:TMP = $env:TEMP
python examples/lzma-wal/build.py target/lzma-wal --clang 'C:\Program Files\AMD\ROCm\6.4\bin\clang.exe'
```

生成物:

| ファイル | 用途 |
| --- | --- |
| `target/lzma-wal/liblzma.wal` | WAL 単体。image ID `lzma`、ABI 1.0 |
| `target/lzma-wal/lzma-demo.wax` | 圧縮して展開し、一致と圧縮率を確認して 42 で終了する WAX v3 |
| `target/lzma-wal/root/lib/lzma.1.wal` | Foundation が実行時に探す配置名 |
| `target/lzma-wal/root/bin/lzma-demo.wax` | Foundation ディスクへ追加する実行ファイル |

ライブラリ単体なら `python foundation/lib/lzma/build.py target/lzma-wal --clang /path/to/clang`。
どちらも最適化の指定は `--optimization=-O1` の形式です（既定 `-O2`）。

`liblzma.wal` は標準の `foundation/boot/build.py` が `/lib/lzma.1.wal` として既定のディスクにも入れます（KAN-58）。
自作ライブラリのように別の WAL をディスクへ足すときは、既存の root overlay を使います。

```sh
python foundation/boot/build.py target/foundation-lzma --root target/lzma-wal/root --clang /path/to/clang -O1
```

生成した `foundation.img` を Foundation のディスクとして起動し、シェルで
`/bin/lzma-demo.wax` を実行します。成功時の終了コードは 42、失敗は 101〜104。
`/bin/lzma-demo.wax; echo $?` で `42` を確認できます。
デモ自身の画面出力はありません。既存の起動手順は `foundation/README.md` を参照してください。

## WAL にする手順

1. ゲストへ移植できる C ソースを選びます。この例では `Z7_ST` で単一スレッド化し、
   host OS、malloc、CPU 固有アセンブリを使わない SDK の経路を選びます。
   `compat/errno.h` は SDK の未使用エラーマクロ用の空ヘッダーで、errno の実装ではありません。
2. `foundation/include/foundation/lzma.h` に固定幅の公開 API を定義します。
   SDK の allocator はアダプター内部だけで作り、関数ポインタを WAL 境界へ渡しません。
3. `foundation/lib/lzma/lzma.exports` に公開名と signature を書きます。
   `--wal --image-id lzma --abi-version 1.0 --exports ...` で SDK とアダプターをリンクします。
   依存する別の WAL はありません。
4. 利用者はヘッダーを include し、`--wax3 --import-wal target/lzma-wal/liblzma.wal`
   でビルドします。`main.c` は SDK のソースを静的リンクせず、2 関数を import します。
5. 実行時は `/lib/lzma.1.wal` を配置します。`liblzma.wal` というビルド名と、
   ローダの探索名 `/lib/<image_id>.<major>.wal` の違いに注意してください。
   libfnd の exec が依存 WAL を読み、カーネルが R30 と import 記述子を束縛します。

WAL はコード RX とデータ RW の 2 領域です。利用側 WAX、スタック、ほかの WAL と合計して
Extension P の 8 領域に収めてください。WAL の静的ポインタ初期化は不可なので、allocator の
コールバックは呼出時に設定します。詳細は `doc/spec_WAX_v3.md` と `tools/llvm-src32/README.md`。

## API とメモリ契約

`cpt_lzma_compress(dst, &size, src, src_size, work, work_size, level, dictionary_size)` と
`cpt_lzma_decompress(dst, &size, src, src_size, work, work_size)` は一括処理です。
`size` は入力時に出力容量、成功時に出力バイト数、失敗時に 0。
失敗後の出力バッファの内容は未定義です。入力・出力・作業領域・size は有効な別領域を渡し、
相互に重ねないでください。`src` と `dst` は長さ 0 でも非 NULL、`size` は書込可能で 4 byte 整列、
`work` は書込可能で 8 byte 整列が必要です。無効なアドレスの検出は行いません。

作業領域は内部 bump allocator に使います。個々の解放はせず、呼出し終了後に全体を再利用できます。
同時呼出しは各々に独立した作業領域を渡します。グローバル allocator や libfnd 依存はありません。
サンプルの level=1、辞書=4096 bytes では 2 MiB の作業領域を用意しています。
デモは作業領域をスタックから確保するため、WAX の `--user-stack-size` を `0x240000` にしています。
level は 0〜9、辞書は 4096〜16777216 bytes。大きい辞書や level では SDK の必要量が増え、
不足時は `CPT_LZMA_MEMORY` になります。一般の入力・設定に対して 2 MiB で足りる保証はありません。
展開時は出力領域を辞書として使う SDK API のため、ヘッダーの辞書サイズ分の別領域を確保しません。
probability table は作業領域に収めます。

返り値は 0 が成功。SDK の数値と同じく、1=不正データ、2=作業領域不足、4=未対応 property、
5=引数不正、6=入力不足、7=出力不足です。SDK が返すその他の非 0 も失敗として扱ってください。
展開先が不足するストリームでは SDK が 1 を返す場合もあり、7 だけを容量不足判定に使わないでください。

形式は 13 byte ヘッダー付き LZMA-Alone (`.lzma`)。圧縮は既知の展開長と終端マーカーを出力し、
展開は既知の長さ、および長さ未指定 (`FF` × 8) + 終端マーカーを受け付けます。
ヘッダーの長さ・辞書サイズは little-endian、WAL とゲストの ABI は big-endian です。
単一ストリームを扱い、余剰データや連結ストリームは拒否します。
LZMA-Alone は checksum を持たないため、任意の破損を検出できる形式ではありません。
XZ Utils の liblzma API 互換、XZ、7z、LZMA2、ファイル I/O、逐次 streaming は提供しません。

## 検証

```sh
python -m unittest discover -s foundation/tests/lzma -v
```

Clang は `CLANG`、testbench は `SRC32_TESTBENCH` で指定できます。
ホストでは Python `lzma` と両方向の互換性、空・乱数・繰返し入力、各圧縮 level、
容量不足・切断・余剰データを確認します。ゲストでは `-O1/-O2` × Scalar/Pipeline で
実際に WAL を呼びます。2 プロセスで同じ WAL を別配置し、作業領域の再利用と
ホスト由来ストリームの展開を確認します。testbench がない場合、そのテストだけ skip します。

2026-10-23 の検証では上記テスト 3 件に加え、root overlay を含むディスクをビルドし、
Cached testbench のシェルから `/bin/lzma-demo.wax; echo $?` を実行して PCG 画面の `42` を確認しました。
