# SGU v3.2 / 34SI22A

SGU v3.1（3WS8PNB×2）の 34SI22 を、上位互換の 34SI22A に置き換える。SGU コア、PCM RAM、34SI22 のレジスタ配置と動作は従来どおり。34SI22A の追加分は [spec_34SI22.md](spec_34SI22.md) §11 に定める。

- 再生チャンネル A／B ごとに 2048 標本（16 ビット符号付き）の FIFO DAC を持つ。
- `REC_A_ADC`（`0x25`～`0x26`）／`REC_B_ADC`（`0x27`～`0x28`）への書き込みで FIFO A／B に標本を積む（上位バイト、下位バイトの順。下位の書き込みで積む）。読み出しは従来どおり ADC の生標本。
- `FIFO_A_COUNT`（`0x29`～`0x2A`）／`FIFO_B_COUNT`（`0x2B`～`0x2C`）で FIFO に残っている標本数を読める。
- `PLAY_FORMAT` の 5 が FIFO DAC。サンプルレートは分周レジスタに従い `48 kHz / (DIV + 1)`。SGU の FIFO DAC と同じく音量（PLAY_VOL）は適用しない（固定 0 dB）。L/R の出力禁止（パン）は適用する。

エミュレータの `config.yaml` で `sgu.version: 3.2` を指定できる（設定省略時は 3.2）。FPGA RTL（src32-sv）は 34SI22A を実装するが、SGU コアは 3WS8PNA のまま（3WS8PNB は未実装）。
