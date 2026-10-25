# Foundation DOOMGeneric の外部ソース移植 (#180)

- DOOMGeneric checkout と WAD はコミットしない。検証した upstream は `e257a09527062bfb986b6e5e0780839b6cbd5b71`。`tools/doomgeneric/build.py` は外部 Makefile から本体のソース一覧を読み、DG backend と WAD backend だけをこちらの実装に差し替える。外部 Newlib の posix profile + pkcl を使用する。
- upstream の `I_Quit` は `ORIGCODE` 無効時に終了しない。ビルド用コピーだけへ `exit(0)` を追加する。Zenity popup もビルド用コピーで除き、stderr とエラー終了は保つ。クローンを直接編集すると再現性を失うので避ける。
- `doom.wax` は約 2.5 MiB。従来の libfnd / proc_spawn の 1 MiB 上限では `FND_E_LIMIT` で起動前に止まる。両方を既存の staging 容量 3 MiB へ広げた。依存 WAL と整列パディングも staging に収まる必要がある。`proc.c` の static assertion と実ゲスト `foundation/tests/doom/limits.c` で単体上限・WAL の溢れ・整列境界・ちょうど収まる場合を検査する。
- FS2 の大きな WAD への繰り返しのランダムアクセスは初期化を非常に遅くする。`w_file_foundation.c` は runtime にディスクから順次読み込んだ RAM バッファを `wad_file_t.mapped` として渡す。バイナリへの WAD 埋め込みではない。1 ファイル 24 MiB まで、nommu 用のヒープ予約は 24 MiB（zone などと共有）。実験用設定は RAM 64 MiB / VRAM 4 MiB。
- DG の CMAP256 バッファは gamma 適用後の palette で RGB888 に変換し、fb0 の y=20..219 へ書く。共有 CLUT は触らない。上下 20 行は黒。部分書き込みを扱い、atexit で fb0 を閉じる。強制終了時は既存のカーネルハンドル解放に依存する。
- upstream の既定キーは Ctrl=`KEY_FIRE`、Space=`KEY_USE`。`KEY_RCTRL` や ASCII space を渡すだけでは射撃・使用にならない。左右の修飾キーを集約し、片側だけの解放で押下を消さない。短い押下／解放は別 scan でエッジを渡す。console の文字入力キューも捨て、プレイ中の文字をシェルへ残さない。
- ホスト検証: `SRC32_DOOMGENERIC_SOURCE` を指定して `python -m unittest discover -s foundation/tests/doom -p test_doom.py -v`。adapter の RGB / 黒帯 / short write / CLUT / 入力 / tick wrap と WAD の EOF・範囲外 offset を実装そのもので検査する。
- 実ゲスト検証: `tools/doomgeneric/run.py --build-only ...` 後、`SRC32_DOOM_BUILD` に出力ディレクトリを指定し `python -m unittest discover -s foundation/tests/doom -p test_guest.py -v`。JIT を指定する場合は testbench も `cargo build --release --features jit --bin src32_testbench` で構築する。古い testbench は JIT 無効のことがある。
- `--allow-running` の testbench は既定で VPU デモ向けの可視画素検査を行う。初期化中に fb0 が PCG を隠していると「VPU command stream produced no visible pixels」になる。DOOM 検証は `--skip-render-checks` を使い、生成 PPM の黒帯・画素色数や明示的な終了 marker / シェルの終了コードを検査する。
- testbench は UART を接続しないので、検証 marker に `fnd_sys_debug_write` を使うと描画後にカーネルの UART 書き込みが bus error になる。frame limit の marker は framebuffer を閉じてから通常の標準出力へ書く。
- 実行検証では必ずディスクを複製して使う。FS2 をマウントしたまま testbench を終了すると元のディスクに DIRTY が残り、ホストの Volume / install が拒否する。`run.py --build-only` が作る未起動ディスクをテストの入力にする。内容検査をせず DIRTY だけを消して使い回さない。
- Windows の既存 WAX ホストテストは MSVC target Clang の `-fPIC` と DLL を解放しない teardown に注意。今回は UCRT GCC を使い、テスト runner 内で `FreeLibrary` 後に temp cleanup して 30 件通過した。既存テストファイル自体は変更していない。
- 実行結果: 指定されたローカル DOOM.WAD（11,159,840 bytes）で E1M1 を testbench/JIT 2400 frames 実行し、320×200 のゲーム画面を目視確認。上下の黒帯がゼロ、画像領域に 152 色あることも確認した。VM / nommu の両構成で `test_real_wad_frames_and_return` が通過（4 回描画、marker、`doom-status=0`）。ホスト adapter テスト、実ゲスト容量境界、既存シェルの基本コマンド検証（VM / nommu）も通過。
