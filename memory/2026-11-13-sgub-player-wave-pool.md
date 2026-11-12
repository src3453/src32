# SGUB Player: Waveform Pool の番号表示と 3WS8PNB の波形長プレビュー

- 各トラックの波形ボックスの上に `WP:<pool ID> L<長さ>[ HOLD]` を出す。`StreamPlayer` が COPY_POOL（wave 型、target 0/1）の転送先チャンネルごとに最後の pool ID を記録し（`wave_source`）、`Engine::channel` がその payload と現在の波形 SRAM 256 byte が一致するときだけ `ChannelView::wave_pool` に入れる。後から WRITE/FILL やチャンネルのリセットで SRAM が変わると `---` になる。v1 の stream には pool がないので常に `---`。
- wave pool entry は v2〜v4 とも厳密に 256 byte、COPY_POOL は 0x100 境界・長さ 256 なので、1 回の COPY_POOL がちょうど 1 チャンネルを埋める前提で記録している。形式を変えたらここも見直す。
- プレビューは `ChannelView::wave_points` が作る。3WS8PNB では SRAM の先頭 `wave_length`（256/128/64/32）サンプルだけが鳴るので、それだけを描く。補間ありは最後のサンプルから先頭へ戻る線分（コアの次点 wrap と同じ）まで描き、HOLD（SAMPLE_CONTROL bit 2、または 3WS8PNA より前）は階段で描く。以前は長さに関係なく `wavetable[i*4]` を 64 点描いていて、短い波形では再生しない SRAM 後半まで描いていた。
- 長さと補間は `S3w2Sound::wavetable_length`／`interpolates`（公開にした）から取り、SGU 版によるレジスタ無視の判定を GUI 側で重複させない。
- 同じ FREQUENCY で長さが半分になると 1 オクターブ上がる（spec_SGU_v3_1.md）。`ChannelView::midi_note` は `freq * 256 / wave_length` で音名と鍵盤の位置を出す。
- テスト: `cargo test --test sgub_monitor`（`channel_view_reports_wave_pool_entry_and_3ws8pnb_length`、`wave_preview_draws_exactly_the_playing_samples`）。GUI の目視は 256/128/64/32/32 HOLD を並べた v4 stream を作り `sgub_player <file> --scale 1` で確認した（波形長を超える SRAM に縞模様を入れておくと、漏れて描かれていないことが分かる）。
