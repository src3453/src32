# SGUB Player の v4 対応（フレーム周期、34SI22、拡張カード表示）

前段は `memory/2026-11-12-issue-203-sgub-player.md`、v4 の形式は `memory/2026-11-12-issue-204-sgub-v4.md`。

- **フレーム周期**: `music::monitor::Engine::tick` は `until_frame`（48 MHz サイクル、符号付き）が 0 以下になったらフレームを 1 つ進め、`frame_cycles()`（VSync は 800000）を足す。区間ごとに `ceil(until/1000)` sample を合成して 1000 × n を引くので、`48000000 / rate` が sample 単位で割り切れない周期でも累積誤差がない（`timer_streams_advance_on_their_frame_period` が 480500 サイクルで 1 秒 = 100 フレームを確かめる）。VSync で 800 sample ずつ呼ぶ場合は以前と同じ順序（進める → 800 合成）で、参照ミキサーとのビット一致テストはそのまま通る。
- **最初のフレーム**: `StreamPlayer` は最初の `advance_vsync` で初期化列と frame 0 を同時に実行する。仕様上タイマ進行は初期化から frame 0 まで 1 周期空くが、プレイヤーは 0 で始める（モニタ用途の差）。
- **デバイスへの振り分け**: 1 フレーム進めるたびに `take_card_writes()` を全部取り出し、target 11 は `Si22Unit::write_register`、3..10 は該当 slot の `CardSlot::write` へ。SGU の書き込みとの相対順はフレーム内で失われるが、デバイスが別なので音には影響しない。34SI22 は SGU と同じ共有 PCM RAM（`new_cores` が返す Rc）を使い、seek では SGU と一緒に作り直す。
- **ミックス順**: SGU 16ch（i32）＋ 34SI22 DAC（i32）→ 1 回だけ飽和 → カード出力を飽和加算。エミュレータのホストミキサーと同じ順。カードは 48 kHz で `render_audio` を呼ぶ（エンジンは常に 48 kHz で合成し、ホストの出力レートとの差は従来どおり無視）。
- **カードのホスト**: `devices::expansion::HostedCard` はバスなしで 1 枚のカードを動かす公開ラッパー（自前の 48 MHz サイクル時計、`advance_cycles` で `cpt32_tick` の期限を処理）。`ExpansionCard` 自体は `pub(crate)` のまま。
- **ID 照合**: 要件表の ID を `HostedCard::read` で読んで比べる（spec の「MMIO player は初期化前に照合」）。不一致・未発見・読み込み失敗・fault のいずれも、その slot への書き込みを捨てて件数を数え、SGU と 34SI22 は鳴らし続ける（テスト `missing_or_mismatched_card_drops_its_writes_but_keeps_playing`）。v3 には要件表がないので slot 0 に 3WS88PWN4（`0x4003F0`、bulk `0..0x400000`）の要件を合成する。
- **カードの種類**は stream ではなく、読み込んだカード自身が返す ID（3HS816 は `0x4F8`、3HS88PWN4 は `0x4003F0` の ASCII `3WS88PWN4`）で決める。仕様名は 3HS88PWN4、カード ID は `3WS88PWN4` で綴りが違う点に注意。パッケージの自動探索は要件表の ID から推した種類のファイル名（`3hs816.cpt32card`／`3ws88pwn4.cpt32card`）で行う。
- **表示はシャドウ**: カードの表示値はプレイヤーが送った書き込みの写し（`BTreeMap`）。MMIO の読み出しはカードによって副作用があり得るので使わない。3HS88PWN4 は `0x400000..0x4003FF` だけを写し、4 MiB の PCM RAM は写さない。Gate への 1 の書き込みでキーランプを 8 フレーム点灯。
- **seek とカード**: カードを `reset` して書き込みを先頭から再送する。S3HS の 4 MiB COPY_POOL は 1 byte ずつの WASM 呼び出しになるので、大きな PCM image を持つ stream では seek が遅い（未対策）。
- **18ch 表示**: SGU 表示は 16 トラックの下に 34SI22 PLAY A／B を 17／18 として並べる（行高 46、ウィンドウ高 864）。34SI22 のメーターはチップ出力の共有値で、チャンネル別ではない（`Si22::tick` は合計しか返さない）。チャンネル別ミュートも同じ理由でなく、クリックはチップ全体のミュート。
- テスト用カードは `tests/sgub_monitor.rs` の WAT（ID を `0x4F8` に置き、Gate が 1 の間 +1000 を出す）。実カードのパッケージは `examples/*/target`（git 管理外）にしかないので、テストは実カードに依存しない。
- 検証: `cargo test --test sgub_monitor`、`cargo test --test furnace_sgub`、`cargo test --test wasm_expansion`。GUI は `furnace_v4_mix.sgub` の repeat flag（byte 6 bit 0）を立てたコピーで目視（元は 1.6 秒で終わる）。
