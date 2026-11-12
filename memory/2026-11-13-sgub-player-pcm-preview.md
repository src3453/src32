# SGUB Player: PCM／SQV4 の全体波形プレビューと再生ヘッド

- PCM トラックの波形ボックスは、PCM_START から PCM_END まで（両端を含む。コアは PCM_END のバイトを鳴らしてからループ／停止する）の符号なし 8 bit を 1 列 1 px で min..max の縦線にして描く（`Engine::pcm_overview`）。列数よりバイトが少ないときも各列に最低 1 バイトを割り当てる。キャッシュはしない。u8 の min/max は自動ベクトル化されるので 1 MiB 全域でも毎フレーム計算で足りる。PCMRAM は stream の WRITE/FILL/COPY_POOL や 34SI22 の録音で変わるので、キャッシュするなら無効化の経路をすべて押さえる必要がある。
- SQV4 も全体波形を出す（`Engine::sqv4_overview`）。コアが PCM_START..PCM_END（END は排他。コアの `initialize_sqv4` と同じ）を検証済み（`sqv4_samples` が Some）のときだけ `sqv4::decode` で全体を復号する。コアの `inspect` は長さの完全一致を要求するので、検証済みなら `decode` も成功する。復号結果は (start, end, 列数) をキーにキャッシュし、毎回エンコード済みバイトのハッシュを取って一致を確かめる（PCMRAM の書き換えで古い表示を出さないため）。ステレオや版違いで復号できないとき、およびコアが最初のサンプルで検証するまでの間は、直近 `VOICE_HISTORY`（256）出力サンプルの自動スケール表示に戻る。履歴は `render_segment` でミュート前の (L+R)/2 を積み、seek で 0 に戻す。
- 再生位置は、波形の背後に塗る背景色のゲージと緑の再生ヘッド線（`PLAYHEAD`）の両方で表す（ユーザー指定）。ループ線（PCM_CONTROL bit 1 のとき赤の `KEY_ON`）とは色を分ける。割合は `ChannelView::pcm_progress` が返す。PCM は byte 長、SQV4 はコアが検証済みにした後の復号サンプル数と loop サンプル（`Channel::sqv4_samples`）を使う。SQV4 は最初のサンプル生成まで未検証なので、ゲージが出ないフレームがある。
- アドレス範囲 `start-end` は波形ボックスの上（Wave の `WP:` と同じ位置）に移した。
- テスト: `cargo test --test sgub_monitor`（`pcm_overview_covers_start_through_end_and_progress_follows_playback`、`sqv4_progress_uses_decoded_length_and_voice_history_records_output`）。SGUB の初期化操作は最初の frame で適用されるので、`Engine::new` の直後はまだ PCM 設定が入っていない。テストでは先に `tick` する。
