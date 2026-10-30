# SGU v3 入力波形テスト

- `examples/sgu-input-scope` は 34SI22 の REC A/B を PCM16、48 kHz、0 dB で録音し、PCM RAM の big-endian 標本を左右別の SGC 線分として描く。直接 ADC レジスタを CPU で反復読みしても、エミュレータでは音声がフレーム単位で進むため実波形を再現できない。
- SGC FIFO は 256 語で満杯時は書き込みを捨てる。FIFO_STATUS の空きを見てから各 word を投入し、保持された画素の消去には SCREEN_FILL を使う。
- BIOS のないディレクトリで README の testbench コマンドを実行し、無音の左右波形の画素を確認した。実音声の表示確認は信号供給が必要。
