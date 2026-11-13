# SGUB Player: 34SI22 タブの削除

- 左パネルの 34SI22 ビュー（`View::Si22`、`draw_si22`、右の `draw_si22_registers` による 0x00〜0x2F のダンプ）を削除した（ユーザー指定）。SGUB は 34SI22 の録音（REC A/B、VOX）を使わず、PLAY A/B は SGU 表示の 17／18 行目と右のレジスタ表に出ているので重複していた。
- 34SI22 のミュートは SGU 表示の 17／18 行目のクリックとミキサーの MUTE で行う（`M` キーはカード表示でだけ効く）。
- 読み込み時の "Tab: device views" はカードがあるときだけ出す（以前は 34SI22 も数えていた）。
- `si22_format` は SGU 表示のトラックとレジスタ表がまだ使う。`Engine::si22()` も同じ。
- 確認: `cargo build --release --bin sgub_player`、`cargo test --release --test sgub_monitor`。GUI は目視していない。
