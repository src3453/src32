# SGUBPlay を Tang Console 実機で鳴らす（2026-10-26〜27 の調査記録・引継ぎ）

目的: `remark_music_s3w2.sgub`（SGUB v2、2603 フレーム、wave pool 16＋PCM 13,376 byte、PCM の FREQUENCY 最大 2813）を、実機（Tang Console 138K）の Foundation `sgubplay` で再生する。
**現状（2026-10-27 追記で解決）: 最後まで実時間（44.0 s／理想 43.4 s）で再生でき、音程も正しい。音程が低く音が欠落した原因は、Gowin V1.9.12.02_SP2 の合成が `hdmi_audio` の別配列（`p_sub`／`p_ecc`）を取り違えて BCH の位置が壊れたこと。`p_body`（64 bit 1 本）にする修正（src32-sv `473d869`）で直った。下の「未解決」節は調査経過の記録で、結論は末尾の「追記」を見ること。**

## 実機で確定した事実

- **SDRAM1 は存在しない。** Tang SDRAM X2 モジュールは W9825G6KH-6 を 2 枚、アドレス・データ・制御線を共有して載せ、FPGA から出る CS は `O_sdram_cs_n`（F21）1 本だけ。2 枚目の CS1 は 74LVC1G04 で `NOT CS0`。`cs_n=0` が U1、`cs_n=1` が U2。`sdram_ctrl` は `cs_n=0` 固定なので U1 の 32 MiB しか使っていない。`IO_sdram1_*` ピンと「SDRAM1 の読み戻しが `007f` の繰り返し」は、この存在しない信号を読んでいたため。2 枚目を使うには `sdram_ctrl` に chip ビットを足し（コマンド時に `cs_n`＝chip、初期化 PRE/REF/REF/MRS を両チップに、リフレッシュを交互に）、CPU と PCM RAM を 1 つのコントローラで調停する。
- そのため **PCM RAM は BSRAM（既定 32 KiB）で実装した**（`soc/pcmram.sv` の `BACKING=2`）。`BACKING=1` の SDRAM1 版は実在しないピン向けで、シミュレーション専用（実機では使えない）。BMC `PCMRAM_SIZE`（0x80060088）は実装サイズ（0x8000）を返し、Foundation `free` が `32 KiB PCMRAM` と表示する。範囲外の書き込みは無視、読み出しは 0。
- BSRAM 64 KiB だと BSRAM 使用率 87% で VDP の読み出しパスが悪化し 48 MHz を割った（place option 3 で 47.966 MHz）。32 KiB（278/340）にして **place option 2 で 48.011 MHz を満たした**。`build_fpga.py` の既定は place option 2。Gowin（V1.9.12.02）は `-verilog_define` 非対応で、`--define` を渡すとビルドが 1 秒で失敗する。マクロで切り替えるのはシミュレーション（iverilog）専用で、Gowin 向けの既定値は `soc_top.sv` に直接書く（`PCM_KB` 既定 32、`PCM_BACKING` 既定 2）。
- PCM RAM は実機で全 32 KiB が書き込み・読み戻しとも PASS（`scripts/check_pcmram.py COM7 --full`）。**3 Mbaud で 32 KiB を 1 回の `R` で読むとホストの USB シリアルが取りこぼす**（約 11 KB で途切れる）。読み出しは 512 byte ずつにする（スクリプトは対応済み）。
- Foundation Tang 版の `dev.sgu` は `ipc.c` で `FND_E_UNSUPPORTED` にされていた。Tang でも通すよう変更済み（未コミット）。`sgubplay.wax` は元から Tang イメージに入っている。
- `sgubplay` の再生時間: 初版は **約 2.1 倍遅かった**（1 フレーム約 35 ms）。原因は 1 操作ごとの seek＋write（1 フレーム平均 9.5 操作＝約 19 回のカーネル呼び出し）。1 フレーム分をまとめて 1 回で書く `FND_SGU_WRITE_RUNS`（0x1303、`foundation/include/foundation/sgu.h` の `fnd_sgu_write_runs`）を追加し、プレイヤー側で連続アドレスの書き込みを 1 つの run にまとめて `sgub_step` ごとに送る形にした。順序は完全に保つ。結果は再生 44.0 s（2603 フレームで 1 フレーム 1 呼び出し）。起動は表示まで約 25 s（`ticks load 199 init 652`＝2.0 s＋6.5 s のほか、プロセス起動に十数秒）で、未最適化。
- バッチの不具合は、長さフィールドの上位バイトを 0 にしていたこと（256 byte の波形コピーが長さ 0 になる）。ホストで `sgub.c` を実ファイルに通す確認用プログラムを作って特定した（`scratchpad/batch_test.c` 相当。同じ方針で再現できる）。エミュレータ上の `python -m unittest discover -s foundation/tests/sgub` は 2 本（`furnace_*_v2`）が失敗していたが、この修正後の再実行はまだ。**次の担当は最初に再実行すること。**
- ボードの音声は USB2 Video キャプチャ（dshow の `デジタル オーディオ インターフェイス (USB2 Digital Audio)`）で録音できる。映像は `video=USB2 Video`（MJPEG 1920x1080 で画が出る。640x480 は黒または真っ黒になることがある）。ROM のままだと VDP が初期化されず黒い。

## 音程が低い問題の切り分け（未解決）

観測:
- 波形テーブルのサイン波（`scripts/sgu_tone.py`／`sgu_tone2.py`、FREQUENCY=F で F Hz のはず）を録音すると、零交差で 220→74 Hz、440→142 Hz、880→244 Hz（比が一定でない）。スペクトルは **50 Hz 間隔のくし形**（F±50k Hz）で、エンベロープに 1〜4 ms の「同じ値の保持」が周期的に現れる。振幅は想定どおり（ピーク約 4000）。
- FIFO DAC のランプ波（毎サンプル +4 が出力で期待される）を録音すると、約 2245 出力サンプルにわたり、連続部分は傾きが約 1/3、30 か所でジャンプして全体の約 3 割しか届かない。
- 他の 15 チャンネルを非アクティブにしても結果は同じ（エンジン負荷ではない）。

切り分けの結果:
- **SGU エンジンは 48 kHz で動いている。** FIFO DAC（FREQUENCY 0）の FIFO_COUNT が 1500 → 684 に 16.96 ms で減った（816 エントリ／17 ms＝48 kHz）。`scripts/sgu_rate.py`。シミュレーションでも 1 フレーム最大 642〜834／1000 クロック。
- **映像は正常**（HDMI として成立し、コンソールが綺麗に出る）。したがってシンクは HDMI モードと認識している。
- **ユーザーも同じ症状を実機の別の再生経路で聞いている**（ユーザー報告）。キャプチャ機器固有とは限らない。
- sim では `tb_hdmi_audio` が 3 フレーム分 3199 サンプルを全て検証し、BCH も独立な多項式除算で一致する。つまり RTL が作るパケットは「私の読んだ仕様」とは一致している。実シンクは一部しか受理していないように見える（パケットの BCH が合っていないと誤訂正で 1/4 程度だけ「有効」になる、という説明が当てはまる）。

次にやること（優先順）:
1. **`hdmi_audio` にデバッグ用カウンタを追加**（`new_smp` 数、パケットに積んだサンプル数、キュー溢れ）。BMC の空きレジスタ（例 0x80060090〜）から ROM の `R` で 2 回読み、時間差で毎秒の値を出す。あわせて **sys 側で 48 kHz の内蔵テストトーン**を選べるようにして（BMC のレジスタ書き込み）、SGU を介さずに HDMI だけを試す。パケット化の切り替え（1 パケット 1 サンプル、1 行あたり最大 2 パケットなど）も実行時レジスタで選べるようにすると PnR（約 10〜20 分）が 1 回で済む。
2. 仕様との差を確認: 参照実装 hdl-util/hdmi の `audio_sample_packet.sv`、`packet_assembler.sv`、`auxiliary_video_information_info_frame.sv` を全文読み、**サブパケットのビット順・HB2（B と sample_flat）・IEC60958 のパリティとチャンネルステータス**を 1 ビットずつ照合する。独立な検算として、実機の既知パケット列（他の HDMI 送信機のキャプチャ）か仕様書のテストベクタと BCH を突き合わせる。
3. ACR（N=6144、CTS=25200）と、実際のピクセルクロック（PLL 21/40×48 MHz＝25.2 MHz）の整合、および ACR の送出頻度（現在は 0 行と 240 行）を見直す。
4. 別のシンク（TV／モニタ）でも同じ症状か確認する。

## 実機の作業メモ

- ビットストリーム: `src32-sv/build/b32-po2/prj/soc_top/impl/pnr/soc_top.fs`（BSRAM PCM RAM 32 KiB、48.011 MHz、LUT 約 37k、BSRAM 278/340）。書き込み: `programmer_cli.exe --device GW5AST-138B --run 2 --fsFile <.fs>`（SRAM のみ、約 24 s）。**Foundation 実行中は ROM が消えている**ので、再ロードの前に書き込み直す。
- Foundation イメージ: `foundation/boot/build.py target/foundation-sgu --platform tang-console --clang 'C:/Program Files/LLVM/bin/clang.exe' -j 6 --root target/music-root`（`target/music-root/music/remark.sgub` が曲）。ロード: `src32-sv/.venv/Scripts/python.exe scripts/uart_load.py COM7 --foundation ../foundation-fpga/target/foundation-sgu/fpga-boot.json --wait 20`（約 70 s）。コマンド送信: `scripts/fnd_cmd.py COM7 "sgubplay /music/remark.sgub"`（10 ms/byte の入力ペーシング）。
- PnR は並行して重い処理（cargo、iverilog 等）を走らせると途中で異常終了することがある。place option 1 と 0 は結果が出ないことが多い。
- 録音: `scripts/sgu_tone.py COM7 440 --record 3 --out build/tone.wav`、解析 `scripts/wav_peaks.py build/tone.wav`。

## 未コミットの変更（コミットしていない）

foundation-fpga: `foundation/bin/sgubplay.c`（バッチ書き込み、所要 tick の表示）、`foundation/include/foundation/sgu.h`、`foundation/kernel/sgu.c`（`FND_SGU_WRITE_RUNS`）、`foundation/kernel/ipc.c`（Tang で `dev.sgu` を有効化）、`foundation/README.md`。
src32-sv: `soc/pcmram.sv`（BACKING 0/1/2）、`soc/soc_top.sv`（`PCM_BACKING`、`PCM_KB`、BMC のサイズ）、`soc/soc_top.sdc`（`clk_pixel`→`clk_serial` を false path）、`soc/build.tcl`、`scripts/build_fpga.py`（place option 既定 2）、`scripts/run_soc_sim.py`（`--pcm`）、`sim/tb_soc.sv`、`sw/soc_test.a`、新規の `scripts/check_pcmram.py` `diag_pcmram.py` `fnd_cmd.py` `sgu_fifo_ramp.py` `sgu_rate.py` `sgu_tone.py` `sgu_tone2.py` `wav_peaks.py`。`soc/hdmi_tx.sv` ほかの VDP 作業は以前から未コミットのまま混在している。
コミット済み: SGU RTL と PCM バッファ（`8e2cc39`、`d81ca1e`）、エミュレータの固定小数点化と周波数上限（`13972c3`、`61fd5c3`）。

## 関連する決定

- PCM の再生周波数上限は FREQUENCY 48000（8 サンプル／内部クロック）、SQV4 は 12000（2 サンプル）。エミュレータにも反映済み（`doc/spec_SGU.md` §2.4）。SQV4 の RTL は未実装。
- 2 枚目の SDRAM の利用（CS 切り替え）は未着手。PCM RAM を 1 MiB にするときの本命。

## 追記: 修正後の実機確認（2026-10-27）

ユーザーが修正（`473d869`、あわせて D3 を島の間ずっと 1 にする仕様の読み直し）を入れて新版 Gowin（V1.9.12.04）でビルドしたビットストリームを実機で確認した。確認は録音を `audio-waveform-check`（`~/.codex/skills/audio-waveform-check`）の手順で画像化して目視し、数値と分けて記録した。

- **波形テーブルのトーン**（`scripts/sgu_tone2.py`）: 440／220／880 Hz がそれぞれ 439.98／219.99／879.96 Hz。スペクトルは単一ピークで他は −50 dB 以下。波形画像（`build/wf_fixed_440.png`）は途切れのない滑らかなサイン波。修正前（`build/wf_before_440.png`）は同じ設定で数 ms の保持と不連続が周期的に入っていた。
- **FIFO DAC のランプ波**: 約 42.7 ms の直線（0→8188、傾きは平均 +4／サンプル。±1 は録音機器の丸め）。サンプルの欠落なし。
- **曲の再生**（旧版 V1.9.12.02_SP2 で同じ修正版を合成して実機に書いたもの）: `sgubplay` が最後まで再生し、録音（`build/song_hw_fixed.wav`）とエミュレータの出力（`foundation-fpga/target/emu-ref/remark_emu.raw`、ゲストの `sgu_music_driver` を `--music-stream` で動かして `--dump-audio`）を比べた。40 秒の長時間平均スペクトル（1/3 オクターブ 100 Hz〜12.8 kHz の 22 帯域）の差が平均 0.39 dB、最大 0.94 dB。RMS は実機 2803／エミュレータ 2912。無音区間の周期はどちらも約 3.46 s で、テンポは一致する。
- 実機はフレームの適用が OS の 100 Hz タイマーに量子化されるため、曲中の短いイベントが最大 40 ms ほどずれて見える（波形画像で確認）。音程や欠落ではない。

**重要: 新版 Gowin（V1.9.12.04）で合成した SoC は Foundation が起動しない。** `build/hdmi-body64-191204-po2` は ROM 経由のテスト（SGU トーン、`soc_test`）は通るが、`uart_load.py` で Foundation を入れてもバナーも画面表示も出ない（4 回再現）。同じ修正済みソースを旧版 V1.9.12.02_SP2 で合成した `build/hdmi-body64-0212-po2`（`clk_sys` 47.967 MHz、place option 2、わずかに未達）では起動し、曲も再生できた。新版で合成した CPU 側の何かが原因と見られるが、未調査（候補: タイミング余裕が 48.003 MHz とほぼゼロ、新版合成の差）。Foundation を使うときは旧版の `--gowin-home C:/Gowin/Gowin_V1.9.12.02_SP2_x64` を使うか、この起動不良を先に調べる。place option 2 の旧版は 47.967 MHz なので、48 MHz を満たす配置オプションの探索も残っている。

注意: Git Bash は `/music/...` を `C:/Program Files/Git/music/...` に変換する。`sgubplay` のようなパス引数を送るときは `MSYS_NO_PATHCONV=1` を付ける。

追加したスクリプト（未コミット）: `scripts/play_and_record.py`（再生しながら HDMI 音声を録音）、`compare_song.py`、`compare_timing.py`、`compare_ltas.py`（録音とエミュレータ出力の比較）。
