# 拡張カードを挿しても Bus の高速経路を止めない

- 症状: CNET カードを読み込むとエミュレータが大幅に遅くなった。Foundation のディスク起動 900 フレーム（release testbench）で、cached-relaxed はカードなし 3.0 秒に対しカードあり 16.0 秒、pipeline はカードあり 18.0 秒。CNET 固有の問題ではなく、カードが 1 枚でも入っていると起きる。
- 原因: `Bus::tick_relaxed_devices`（relaxed バッチ）、`quiet_cycles`（スピン早送り）、`native_idle_budget`（JIT のアイドル予算）が `has_expansion_cards` で一律に無効になっていた。そのため毎サイクル `advance_system_cycles(1)` で 8 スロットの走査と IRQ の再サンプリングが走り、スピンループも早送りされなかった。
- 修正: 3 か所から `has_expansion_cards` の条件を外した。カードの期限は以前から `cycles_until_next_event` に入っていて、タイマーの期限と同じように窓を終わらせる。カード MMIO は timing-insensitive ではないので、アクセス前に relaxed バッチを flush する（カードが読む `current_cycles` は最新になる）。カードの IRQ は guest 呼び出しのときにしか変わらない。結果は cached-relaxed がカードあり 3.5 秒、pipeline が 6.6 秒（カードなし 5.7 秒）。
- 契約: ネットワークフレームの受信はホスト入力と同じ非同期の扱い。バッチや quiet 窓の途中に届いたフレームは、窓の終わり（flush か次のイベント）で `cpt32_net_rx` に渡す。窓の開始時点ですでに pending なら `next_event_cycle` が現在 cycle を返すので、窓は 0 になる。HALT の早送り（`cpu.rs` の `cycles_until_next_event`）はもともとこの扱いだった。
- テスト: `src/bus.rs` の `relaxed_device_tests` に、期限付きの偽カード（`DeadlineCard`）を使う `installed_cards_keep_relaxed_batching_until_their_deadline` と `quiet_window_ends_before_a_card_deadline` を追加した。以前の「カードがあるとバッチしない」テストの後半は削除した。
- 検証: `cargo test --lib relaxed_device_tests`、`cargo build --manifest-path examples/wasm-cnet/Cargo.toml --release --target wasm32-unknown-unknown` のあとで `cargo test --test cnet --test wasm_ui --test wasm_expansion --test wasm_psg --test cpt32pkg_runtime --test sgub_monitor --test spin_fast_forward --test timing_insensitive_mmio --test cpu_relaxed --test cpu_select --test cpu_pipeline`、`python -m unittest discover -s foundation/tests/cnet`。
- `cargo test --features jit --test cpu_jit` の `fallback_memory_arithmetic_short_mode_and_halt_preserve_cycles` は、この変更の前から失敗している（831 != 319）。
- `cargo fmt -- src/bus.rs` を実行すると、crate 全体の未整形ファイル（`cpu_pipeline.rs`、`sgu.rs`、`si22.rs`）まで書き換えられる。単一ファイルの整形には `rustfmt --edition 2024 src/bus.rs` を使う。
