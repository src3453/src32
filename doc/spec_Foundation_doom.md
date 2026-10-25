# Foundation DOOMGeneric 移植

DOOMGeneric を外部ソースからコンパイルし、Foundation のシェルから `doom` として起動します。ゲーム本体のソース、Newlib、WAD、生成した実行形式・ディスクはリポジトリに含めません。

## ビルドと起動

Clang、Python + llvmlite、Rust/Cargo と外部 Newlib のソースが必要です。Newlib の取得は [外部 Newlib の案内](../examples/newlib-c/README.md) を参照してください。ここでは限定版ではなく、既存の `--newlib-profile posix` と pkcl を使用します。

最初に [DOOMGeneric](https://github.com/ozkl/doomgeneric) を Git 管理外の場所へクローンします。

```powershell
git clone https://github.com/ozkl/doomgeneric.git target/doomgeneric
git -C target/doomgeneric checkout e257a09527062bfb986b6e5e0780839b6cbd5b71
python tools/doomgeneric/run.py --doomgeneric-source target/doomgeneric --newlib-source target/newlib-work/newlib-4.5.0.20241231 --wad C:/Users/y2k34/Downloads/doom/DOOM.WAD
```

`--wad` には自分で用意した WAD を指定します。指定ファイルは読み取るだけで、生成ディスクの `/doom.wad` にコピーします。`target/foundation-doom/` に起動イメージ、64 MiB の FS2 ディスク、`doom.wax`、実行用設定を作ります。`--build-only` で GUI 起動を省略できます。`--clang PATH`、`-j N`、`--out DIR` も使用できます。生成物は Git 管理外に置いてください。

`run.py` は毎回ディスクを再生成します。セーブや設定を保持して同じディスクを再利用する場合は、初回ビルド後、エミュレータだけを起動します（Windows）：

```powershell
Push-Location target/foundation-doom/run
../../release/cpt32.exe ../boot/foundation.wax
Pop-Location
```

起動した Foundation のシェルで入力します。

```text
doom -iwad /doom.wad
```

DOOM のオプションはそのまま渡します。例：最初のマップで開始するには `doom -iwad /doom.wad -warp 1 1`。起動引数やセーブ先の扱いは外部エンジンに従います。書き込み可能な cwd で実行してください。

既存の Foundation ディスク向けに実行形式だけを作る場合：

```powershell
python tools/doomgeneric/build.py --doomgeneric-source target/doomgeneric --newlib-source target/newlib-work/newlib-4.5.0.20241231 --fnd-wal target/foundation-boot/root/lib/fnd.1.wal --out target/foundation-doom
```

`/bin/doom.wax` と対応する `/lib/fnd.1.wal`、WAD をディスクに配置します。古いカーネル／libfnd の実行形式上限は 1 MiB なので、今回の 3 MiB 上限を含む Foundation を再ビルドしてください。WAX と WAL の staging 合計は引き続き 3 MiB 以下です。

## プラットフォーム契約

- 移植コードは `tools/doomgeneric/doomgeneric_foundation.c`。upstream の DG API を実装します。WAX v3 の再配置可能なアプリで、pkcl の `pkcl_start` から通常の `main(argc, argv)` を呼びます。
- 設定は Main RAM 64 MiB、VRAM 4 MiB。スタック 128 KiB、nommu 用のヒープ予約 24 MiB、エンジンの既定 zone は 6 MiB。親シェルによる WAX/WAL の読み込みにも RAM が必要です。nommu では WAD バッファと zone などの合計が予約したヒープに収まる必要があります。
- 映像は 320×200、320×240 の画面の y=20..219 に等倍表示します。DOOMGeneric の `CMAP256` バッファと現在の gamma 適用済み palette を RGB888 に変換し、`/dev/fb0` に書き込みます。SGC/GP と共有する CLUT は変更しません。部分書き込みを処理し、正常終了・エラー終了では framebuffer を閉じてコンソールへ戻します。強制終了時の解放はカーネルのハンドルクローズ処理が行います。
- 入力は console の HID usage 状態。方向キーが移動／旋回、Ctrl が射撃、Space が使用、Shift が走行、Alt が横移動、`,`／`.` が左右の横移動、数字が武器、Enter が決定、Esc がメニュー、Tab が地図です。文字キー・F1〜F12 も渡します。左右の修飾キーは同じ DOOM キーに集約し、短いタップも押下と解放にします。
- 時刻は Foundation の monotonic tick を ms へ変換します。sleep は tick 単位に切り上げます。独自に VSync/IRQ を生成しません。
- 標準 C のファイル操作・確保・stdio は既存の pkcl/Newlib に任せます。`w_file_foundation.c` は WAD を起動時に一度だけ順次読み込みし、RAM 上のバッファをエンジンの `wad_file_t.mapped` に渡します。FS2 のブロックチェーンに対する繰り返しのランダムアクセスを避けるためです。1 ファイルの上限は 24 MiB で、必要な RAM を確保できない場合は開くのに失敗します。WAD 全体を実行形式に埋め込みません。
- 音声・音楽・マウス・ネットワークは未対応です。35 Hz のゲーム進行を使いますが、描画とエミュレーションの速度によって実際の FPS は下がります。

## upstream への適応と検証

検証対象 upstream は `e257a09527062bfb986b6e5e0780839b6cbd5b71`。ソース一覧は upstream の Makefile から読み、X11 backend と stdio WAD backend を Foundation の実装に差し替えます。クローン自体は変更しません。ビルド用 `i_system.c` のコピーで、終了 hook 後に戻ってしまう `I_Quit` に `exit(0)` を加え、エラー時の Zenity popup 呼び出しを除きます。エラーの stderr 出力と終了は保持します。適応対象が変わった場合はビルドを止めます。使用した revision は `sources.json` に記録します。

ホスト検証（外部 checkout のヘッダーのみ使用、WAD 不要）：

```powershell
$env:SRC32_DOOMGENERIC_SOURCE = "$PWD/target/doomgeneric"
python -m unittest discover -s foundation/tests/doom -v
```

実際の adapter を mock デバイスに接続して RGB の順序、上下の黒帯、共有 CLUT の保持、部分書き込み、tick wrap、sleep の切り上げ、短いタップ、左右 Ctrl の集約、close を検査します。WAD backend の読み込み・EOF・範囲外 offset も検査します。

作成済みディスクを使う実行テストは次のとおりです。WAD はディスクから読み、テストごとにディスクを複製します。容量境界、実 WAD の描画と終了、最初のマップの画素を検証します。`FND_MM=nommu` の検証には、その設定で Foundation をビルドした別の出力ディレクトリを指定してください。

```powershell
$env:SRC32_DOOM_BUILD = "$PWD/target/foundation-doom"
$env:FOUNDATION_TEST_CPU = "jit"
cargo build --release --features jit --bin src32_testbench
python -m unittest discover -s foundation/tests/doom -p test_guest.py -v
```

`doom -iwad /doom.wad -fnd-frames 4` は描画を指定回数行った後、コンソールに `doom: frame limit reached` を出してコード 0 で終了する検証用オプションです。通常のプレイでは指定しません。

DOOMGeneric のリンク済み成果物を配布する場合は upstream の GPL-2.0（またはその後の版）と使用した Newlib のライセンス条件に従ってください。WAD は別のゲームデータであり、エンジンのライセンスとは別に扱います。
