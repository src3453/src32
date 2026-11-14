# sgub_player: Noise／PCM／SQV4 でも鍵盤とノート名を表示する

- `ChannelView::midi_note`（`src/music/monitor.rs`）は Wavetable に加えて Noise・PCM・SQV4 でも音程を返す。トラックの `NT:` 表示と鍵盤のハイライトは両方ともこの関数を使う。
- 換算は「FREQUENCY を Hz とみなす」。根拠は music editor の書き込み規則（`src/music/stream.rs` の `frequency()`）：Noise は Wavetable と同じ A440 基準で Hz を書き、PCM は `sample_rate/32 × 2^((note-base)/12)` を書く。レジスタには base note が残らないので、PCM の表示は「標本の元レートが FREQUENCY×32 Hz のとき A-4 = 440」という相対表示で、楽器の base note とは一致しない場合がある。
- PCM／SQV4 は再生と同じく `PCM_FREQ_MAX`（48000）／`SQV4_FREQ_MAX`（12000）で頭打ちにしてから換算する（読み戻し値は書いた値のまま、`doc/spec_SGU.md` 2.4）。
- FIFO DAC と DMA PCM は音程を持たないので `None`（`---`）。FREQUENCY=0 も `None`。
- 34SI22／3HS88PWN4 の鍵盤は別関数で、この変更の対象外。
- 検証: `cargo test --lib music::monitor`（`noise_and_pcm_show_frequency_as_pitch`）。
