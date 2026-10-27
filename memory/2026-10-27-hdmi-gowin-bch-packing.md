# Tang HDMI 音声: Gowin 合成後のパケット破損

- 引継ぎ `2026-10-27-sgubplay-on-tang-hardware.md` と D3 followup の続報。Foundation の seek/write batching とは別の原因。
- `src32-sv/soc/hdmi_audio.sv`: 56-bit data と 8-bit BCH を別の配列に置く RTL は Icarus で正常でも、Gowin V1.9.12.02_SP2 合成後には control packet の並びが違った。static control writes と indexed audio writes を併用する配列の統合で、BCH が後続 subpacket の低位 byte に現れる。
- ACR の期待値 `1d00180070620000`（64 bit）は、合成後 sub0 `0000180070620000` / sub1 `181800706200001d` / sub2 `0000706200001d00` / sub3 `1d706200001d0018`。header `4a000001` は一致。AVI / Audio InfoFrame でも BCH・checksum が破損。
- **修正:** `p_body[0:7]` を 64 bit にし、`{ecc56(sub), sub}` を一回の代入で保持する。HDMI の送出頻度、N/CTS、音声サンプル生成は変えていない。
- 実機比較: `486tang` の HDMI モジュール一式を差し込むと正常。自作版も上記修正だけで、左 1 kHz・右 2 kHz の連続方形波になった。録音 `src32-sv/build/hdmi-diag-body64.wav` を PNG にし、80 ms / 8 ms の画像を実際に開いて確認済み。
  - 左の半周期: 24 samples 1996 回、25 samples 3 回。右: 12 samples 3996 回、13 samples 3 回（各 1 秒間）。修正前は左 70/71、右 35/36 が最頻値で、数 ms の保持が周期的にあった。
  - 同じキャプチャーボードで正常な参照版・修正版が録音できた。ユーザーの別端末でも修正前の症状は再現した。端末変更だけの結果と、同じ送信条件に近づけた実装 A/B を区別して記録する。
- 検証:
  - `src32-sv/scripts/run_hdmi_audio_sim.py`: 2116 packets / 3199 samples PASS。
  - 新規 `scripts/run_hdmi_audio_synth_sim.py`: Gowin で packetizer を合成し、同じ TMDS decoder testbench を GW5A primitive models + GSR で実行。修正前 source では subpacket BCH / ACR copies / InfoFrame checksum が FAIL。修正後 `build/hdmi-synth-body64` は 529 packets / 799 samples PASS（800 sent、末尾の in-flight sample は許容）。
  - RTL のみのテスト成功では再発を検出できない。packet storage/generation の変更では合成後の検証を行う。
- 除外できた候補: D3=1 修正のみ、async sample CDC 除去、ACR を約 1 kHz に増加、4 samples 固定、HDMI clock の pclk 直結はいずれも症状を解消しなかった。pixel clock は UART 計数で system 48,000,000 clocks に対して 25,200,000 ticks（4 回一致）。
- 既存実装の比較・blob SHA・診断 build 手順は `src32-sv/doc/hdmi-audio-debug.md`。参照コードは `build/486tang-hdmi-ref` にのみ置き、production には取り込んでいない。
- ユーザーが `C:/Gowin/Gowin_V1.9.12.04_x64` へ更新したため、旧版で起動した全体 SoC の `build/hdmi-body64-po2` は今回の gw_sh プロセスだけを確認して停止した（211 s、ユーザーのツール更新に伴う意図した中断。合成失敗と解釈しない）。
- **新版 V1.9.12.04:** 修正前の別配列版も `build/hdmi-synth-191204-before` で合成後検証 529 packets / 799 samples PASS。今回の再現ケースでは新版で破損が解消している。64-bit 保存も `build/hdmi-synth-191204-body64-retry` で同じ検査 PASS。最初の新版 body64 build はログ空・起動直後 exit 1 だったが、新しい出力先での再試行は正常。64-bit 保存はパケットの一体性を明確にするので維持。
- `scripts/build_fpga.py` / `build_hdmi_audio_diag.py` に `--gowin-home` を追加。合成後検証スクリプトは同オプションを compiler と primitive library の両方に反映。共通 wrapper (`../.claude/skills/gowin-build/scripts/gowin_build.py`) も新版を優先するよう変更した。
- V1.9.12.04 では `set_option -gen_text_timing_rpt 1` が unknown option。初回全体 build の起動直後終了はこれが原因だった。wrapper からこの指定を外し、通常の HTML timing report を使う。新版で `build/hdmi-diag-body64-191204` の PnR / timing 取得は PASS（50 s、1571 LUT+ALU、pixel Fmax 74.905 MHz）。
- 新版で全体 SoC を `build/hdmi-body64-191204-po2` にビルド中。完成後、SRAM に書いて ROM からの SGU tone、Foundation の `sgubplay /music/remark.sgub` を確認する。診断版を起動したまま Foundation ローダを実行しない（boot ROM がない）。
