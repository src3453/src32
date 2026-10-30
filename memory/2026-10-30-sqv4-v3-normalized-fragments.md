# SQV4 版 3: フラグメント正規化と独立復号

- 形式（`doc/squeezvox/squeezvox4_spec_stub.md`）: 23 標本ごとの `[Volume:8][Index:8][codes 23 個]`。コード領域はバイト境界まで 0 でパディングする（H 14 バイト、L/L+ 11 バイト）。各フラグメントは予測値 0・初期インデックス `Index` から始まるので、前のフラグメントなしに復号できる（ストリームの途中から参加できる）。FirstSample フィールドは持たない（ユーザー判断）。
- エンコーダは、ピーク `|x|` から `Volume = ceil(peak*255/32767)` を決め、`x*255/Volume` に正規化し、初期インデックス 0..88 を全探索して、出力領域（音量適用後）の二乗誤差が最小のものを選ぶ。音量 0 のフラグメントは index 0・コードすべて 0。
- 復号の音量適用は版 3 だけ**最近接丸め**（`(|p|+127)/255`）。版 1/2 は 0 方向切り捨てのまま。切り捨てのままだと、小振幅（-60 dB）の H で正規化した方が誤差が大きくなった（RMS 0.46→0.54）。丸めにすると 0.13 まで下がる。
- 正規化の効果は L/L+ で大きく（-18 dB の振幅で RMS 171→136、182→132）、H では小さい。IMA のステップ表はもともと対数なので、極小振幅を除けばスケールにほぼ依存しない。版 2 から版 3 の改善の多くは、フラグメントごとのインデックス探索から来ている。
- 版の扱い: SQV4 版 3 は SGU v3 の変更の一部（チップリビジョンは変えない）。SGU v3 の WAVEFORM 4 と 34SI22 は版 3 だけを、SGU v1/v2/v2.1 は版 1/2 だけを再生する（`s3w2.rs` の `initialize_sqv4`）。`set_version` で SQV4 キャッシュを無効化する。ファイル用 `sqv4::decode` は版 1〜3 を読む（コンバータのプレビュー用）。エンコーダは版 3 だけ。
- 34SI22 の録音は 23 標本をバッファし、フラグメント完成時にまとめて書く（`encode_fragment` を共有するので、ファイルエンコーダとバイト一致）。RTL（src32-sv）は版 2 の逐次符号器のままで、初期インデックス 89 通りの探索は 1 標本周期の予算（約 1000 クロック）に収まらない。RTL 側は別途設計が必要。
- 波形確認（audio-waveform-check）: 0〜-60 dB の減衰トーンで SNR は H/L+ が約 29 dB、L が約 24〜26 dB で、レベルによらずほぼ一定。予測値を毎回 0 から始めるため、フラグメント境界の誤差の跳びは内部の約 1.4〜1.6 倍。孤立したクリック（±30000 の 2 標本）は H でピーク約 1/3 に鈍り、同じフラグメントの直前の無音に、最大約 3700 のリンギングが出る。
- `cargo fmt -- <file>` はクレート全体を整形する。対象ファイルだけ整形するなら `rustfmt <file>` を使い、無関係な差分をコミットしない。
- 検証: `cargo test --lib --release devices::sgu`（`every_fragment_decodes_on_its_own`、`normalization_reduces_quantization_error_for_quiet_fragments`、`sqv4_file_version_must_match_the_sgu_generation`、`sqv4_v3_loop_starts_at_an_independent_fragment`、34SI22 の `sqv4_recording_matches_the_file_encoder_and_plays_back`）。ImHex: `imhex.exe --pl format X.sqv4 tools/imhex/sqv4.hexpat out.json`。
