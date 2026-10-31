# DOOMGeneric を Tang Console 実機（16 MiB）で動かす

- 実機の Foundation に `/dev/fb0` はあるが INDEX8 だけ。doom アダプタは RGB888 が `FND_E_NO_MEDIA` なら INDEX8 + CLUT に切り替える。fb0 を `DG_Init` で開くと、初期化中のコンソール出力（PCG の text/FG/BG 面は bitmap の先頭 12 KiB と同じ VRAM）が画素として混ざり、未初期化 VRAM も見える。最初の `DG_DrawFrame` まで開かない。
- UART 入力は `CON_KEY_STATE` に現れない。`fnd_con_pollkey` の文字／矢印を doom キーに変換し、200 ms の擬似押下にする（端末にキー解放はない）。
- RAM は 16 MiB（空き約 9.9 MiB）で、11 MB の WAD は載らない。5 MiB 超は stdio を通さず `open/lseek/read` でストリーム。stdio 経由だと lump ごとの IPC が増える。
- FS2 の連鎖は各ブロックの先頭に next があり、ランダムアクセスは先頭から辿る必要があった（ヒントは inode ごとに 1 件で後方シークは常に先頭から）。`fs2_vol.extents`（連続した run の索引、64 件）を追加し、既知の run 内なら辿らずに引く。run は連続ブロックを実際に観測したときだけ伸ばし、truncate／delete では `hint_drop` で消す。初回に WAD 全体を 1 回辿る（約 11 MB を読む）コストは残る。
- `kmem_copy` / `fs_copy` は、コピー元とコピー先の 4 バイト境界が違うとバイトコピーだった。アライン済みワード読み + シフト合成を追加した（ターゲットはビッグエンディアン。fs ホストテストは x86 なので `__BYTE_ORDER__` で分岐）。実機の WAD 読み込みは変わらず、コピーは主因ではなかった。
- `fb_call` の VRAM 書き込みはワード単位にした（外部 VRAM は 1 アクセスごとに SDRAM 書き込みと VDP コピー受理を待つ）。フレームの fb0 書き込みは約 0.22 秒。
- 実機の計測（11.2 MB の DOOM.WAD、1-1、最終版）: init 約 138 秒、WAD 読み込み 1364 回で 約 115 秒（約 85 ms/回。stdio から open/lseek/read にして約 25% 短縮。連続領域の索引で 232 秒 → 175 秒）、ゲーム中 約 2.2 秒/フレーム。エンジンの計算が約 1.9 秒。エミュレータ（cached-relaxed）は実機の約 1/6 の時間で、傾向は同じ。IPC の空往復が約 1.9 万サイクル（doc/foundation_ipc_performance.md）で、1 lump あたりの往復数を減らすのが次の課題。
- 計測のやり方: `-fnd-frames N` が `/doomstats.txt` を書く。fb0 を開いている間のコンソール出力は、終了時の復元で消えるのでファイルに書く。
- 実機に載せる手順は doc/spec_Foundation_doom.md「Tang Console 実機」。ロード直後はシェルの入力が欠けることがあるので、空行を送ってからコマンドを入れる。
- Python 3.15 は UTF-8 モードが既定で、日本語 Windows のリンカ出力（cp932）を `text=True` で読む既存テストが失敗する。`python -X utf8=0 -m unittest ...` で実行する。
- 未検証: HDMI のノイズはキャプチャーボードの発熱が原因だった（ボードの不具合ではない）。キー入力の実機での操作感、シェルテストの nommu 構成。

## 追記: -fnd-light（描画を軽くする）

- この移植では設定ファイル読み込みが `#if ORIGCODE` で無効なので、`-config` で `detaillevel`/`screenblocks` は変えられない。アダプタの `-fnd-light` が `DG_Init` で `detailLevel=1`、`screenblocks=8` を設定し `R_SetViewSize` を呼ぶ。
- 実機（40フレーム、`-warp 1 1`）: 通常 84.2 秒（約2.1秒/フレーム）→ light 57.4 秒（約1.4秒/フレーム）。起動 ~136 秒、WAD 読み込み ~116 秒は変わらない。
- UART 入力直後は先頭の数文字が欠けることがある。`-mb 4` が欠けるとゾーン確保に失敗するので、空行を先に送る。
