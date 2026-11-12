# SGUB Player（演奏モニタ付き単体プレイヤー）

- `src/bin/sgub_player.rs` は winit + wgpu + imgui の GUI 部分だけ。再生・解析は `src/music/monitor.rs` の `Engine` にあり、ウィンドウなしでテストできる。GUI の初期化は `music_editor` と同じ手順の複製。
- `Engine::tick` は `S3w2Sound::clock`（チャンネル別出力）を足し合わせてメーター・ミュートを実現する。ミュートなしの合計は `clock_mixed_into_i32` → `saturate_output` とビット一致しなければならない（1 チャンネルの 4 サブティック合計は i16 で飽和しない前提）。`tests/sgub_monitor.rs` の `per_channel_mix_matches_reference_mixer` がこれを守る。SGU のミキサーを変えたらこのテストで確認する。
- フレーム契約はエディタ／ゲストドライバと同じ: 1 tick = `advance_vsync` 1 回 → 合成。レジスタ更新は 2 コアの合成より先。
- seek は新しい `StreamPlayer` とコアを作り、`tick` と同じく 1 フレーム 1 回 `advance_vsync` を呼ぶ（音声は合成しない）。非 repeat の stream で終端を越える seek は最後の stop-all フレームまで適用して `finished` になる。最初の実装は `frame_count` で打ち切って stop-all が抜け、音量が残っていた。
- `StreamPlayer` に読み取り専用の `version`／`frame_count`／`frame_index`／`repeats`／`is_finished` を追加した。再生の意味は変えていない。
- ノート名は Wavetable のみ（FREQUENCY 1 = 1 Hz、A4 = 440）。PCM／SQV4／Noise／FIFO は音程を表示しない。
- v3 のカード書き込み（target 3）は `take_card_writes` で捨てて件数だけ表示する。S3HS を鳴らすには WASM カードのホストが要る（未対応）。
- imgui 既定フォント（ProggyClean）には ▶ ‖ ← → などの記号がなく `?` になる。表示文字は ASCII に限る。
- `cargo fmt -- <file>` はクレート全体を整形して無関係なファイル（`bus.rs` など、未整形のまま入っている）を書き換える。新規ファイルだけ整形するときは `rustfmt --edition 2024 <file>` を使う。
- 検証: `cargo test --test sgub_monitor`、`cargo test --lib music::monitor`。GUI は `cargo run --release --bin sgub_player -- tests/fixtures/furnace_pcm_v2.sgub` で目視。
