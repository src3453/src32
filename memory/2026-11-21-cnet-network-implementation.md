# CNET ネットワーク第 1 版の実装

- ホストの物理層は `src/devices/expansion/network.rs`（loopback と UDP のセグメント、`network_segment` 設定の解析）。`expansion.rs` に `net_info`／`net_send`／`net_recv` の import と任意 export `cpt32_net_rx`、manifest の `capabilities` は `expansion/package.rs`。スタック本体は `examples/wasm-cnet`（SDK の `net_*` と `Card::net_rx` を使う）。
- **Bus の落とし穴**: 受信待ちのカードは `next_event_cycle()` が「現在 cycle」を返す。以前の `advance_system_cycles` は全カードの最小期限を取ってから `> now` で絞っていたため、受信待ちのカードが他カードの後の tick 期限を隠し、長い advance で tick が 1 回しか走らなかった。`next_expansion_event_cycle_after(now)` でカードごとに絞ってから最小を取る。`run_due_expansion_events` は、あるカードのイベントが前のスロットへフレームを積んだ場合に同じ cycle で最大 8 回繰り返す。
- 配送はホストの UDP 受信スレッドがポートのキューに積み、`pending` フラグで Bus に知らせる。guest 呼び出しは Bus のイベント時点だけで、`net_send` の中で他カードを呼ばない（再入しない）。
- マルチキャストは同じ PC の複数エミュレータで同じポートを共有するため `socket2` で `SO_REUSEADDR` を付ける（std の `UdpSocket` ではできない）。マルチキャストで戻る自分のパケットは UDP ヘッダの SENDER（起動ごとの乱数）で捨てる。
- 未接続のカードではホストはソケットを開かない。セグメントは参照されたときに初めて開く。
- SDK の `export_card!` はすべてのカードで `cpt32_net_rx` を export するようになった。ホストは接続済みのカードにしか呼ばないので、既存カードには影響しない。
- `tests/cnet.rs` のカード系テストは、ビルド済みの `examples/wasm-cnet/target/wasm32-unknown-unknown/release/cpt32_wasm_cnet_card.wasm`（または `CPT32_CNET_CARD_WASM`）が必要で、ないと panic する（skip しない）。CI の rust-tests ジョブで先にビルドしている。
- 検証: `cargo build --manifest-path examples/wasm-cnet/Cargo.toml --release --target wasm32-unknown-unknown` → `cargo test --test cnet`。回帰確認に `cargo test --test wasm_expansion --test wasm_psg --test cpt32pkg_runtime --test sgub_monitor --test spin_fast_forward --test timing_insensitive_mmio --test cpu_relaxed`、SDK は `python -m unittest discover -s sdk/wasm-expansion-card -p 'test_*.py'`。
- 未対応: Foundation／ゲスト側ドライバ、ルーティング、ストリーム、名前解決、TCP/IP ブリッジ。GUI の確認は、詳細ウィンドウに「Network segment」行を足しただけで、実機では見ていない。
