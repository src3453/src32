# UART のホスト出力は失敗してもエミュレータを落とさない

- PeC UART（`src/devices/pec/serial.rs`）は送信バイトをそのままホストの stdout へ複製する。以前は `write_all(...).unwrap()` で、Windows のコンソールモードの stdio が非 UTF-8 のバイトを `InvalidData` で拒むと cpt32 が panic した（Foundation が不正な文字を出力したとき）。
- 契約: ホスト側への複製の失敗はゲストに見せず、エミュレータも落とさない。`write_host_byte` が `InvalidData` のバイトを U+FFFD で表示し、ほかのエラー（パイプ切断など）は無視する。バイトはそのまま書く（`as char` で再エンコードしない。UTF-8 の多バイト列は Rust の Windows stdio が write をまたいで組み立てる）。
- ホストの I/O を `unwrap()` するとゲストの出力しだいでエミュレータが落ちる。ゲストが内容を決めるホスト出力では I/O エラーを握りつぶすこと。
- 検証: `cargo test --release --lib devices::pec::serial`（コンソールを模した厳格な writer と壊れたパイプで確かめる）。
