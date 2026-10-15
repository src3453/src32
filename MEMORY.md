## 作業で得た知識・注意点

- WAX v1 (doc/spec_WAX.md): .wax/.wal は同一 64B ヘッダのコンテナで種別のみ違う。ロード先は
  section ごとではなく base ひとつ + entry_offset、debug は NOLOAD の付録。ヘッダ複数バイト項目は
  little-endian。ローダはマジック sniff で素朴 .bin (base 0/PC 0) と両対応し、CPT32PKG manifest は不変。
- `cargo test` (lib) は `src/devices/sgu/sqv4.rs` の `include_str!("../../../../s3w2_core/tests/...")`
  がリポジトリの親ディレクトリ側 `s3w2_core` を指し、存在しないためコンパイル失敗する。wax 単体は
  空スタブを親側に一時配置して `cargo test wax::` で通した。スタブは必ず削除すること。
- testbench の `--expect-mem-u8` で `ST` (32-bit ワード格納) を確認する際は big-endian のため
  LSB は +3 番地に来る (例: 42 を 0x2000 に ST したら 0x2003=42 で確認)。
