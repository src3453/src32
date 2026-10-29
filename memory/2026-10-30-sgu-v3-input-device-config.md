# SGU v3 の入力デバイス選択

- GUI エミュレータの `audio.input_device` は OS の録音デバイス名との完全一致で選択し、省略時は既定入力を使う。同名の複数デバイスや未検出は警告して無音にする。
- 入力ストリームはゲストによる ADC 初回使用時に開く。設定は起動時に読み、カートリッジ交換後も維持する。
- Windows 上で `Voicemeeter Out A1 (VB-Audio Voicemeeter VAIO)` のストリームを開き PCM8 録音完了を確認したが、取得した 4096 標本は無音だった。実音声の経路確認には VoiceMeeter への信号供給が必要。存在しないデバイス名は拒否された。
- focused check: `cargo test --bin cpt32 audio::tests -- --nocapture`。
