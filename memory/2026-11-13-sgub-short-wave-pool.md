# SGUB exporter: 3WS8PNB の短い波形も wave pool で共有する

- Furnace fork（`~/remote/furnace`、`src/engine/export/sgub.cpp`）の exporter は、完全一致の 256 byte 波形を `wavePools` の map で既に 1 entry にまとめていた。重複が残っていたのは 3WS8PNB の短い波形で、`updateWave()` は `wantWidth()`（32/64/128）byte しか書かないため pool に入らず、Wave synth の各ステップが毎 tick 32〜128 個の WRITE になっていた。
- 修正後は、チャンネル先頭からちょうど 32/64/128/256 byte 連続する SRAM 書き込みを 1 つの波形として扱い、256 byte まで繰り返し並べた type 1 entry を COPY_POOL する。wave entry は 256 byte、COPY_POOL は `0x100` 境界・長さ 256 のまま（`doc/spec_SGUB_v2.md` の型規則は変えていない）。
- 後半を繰り返しで埋めてよいのは、`doc/spec_SGU_v3_1.md` で `+len..+0xFF` は保持されるが再生に使わないため。直前の SRAM の後半をそのまま残す方式にすると、同じ短い波形でも履歴によって別 entry になる。Furnace は波形長を変えるとき必ず新しい幅で波形を書き直してから SAMPLE_CONTROL を書くので、後半が鳴ることはない。
- 旧リビジョン（chipType 0/1）の Wave synth は 1 ステップで 256 byte 中 1 byte ずつ変わる本当に別の波形で、数は減らない（間引いて表示すると同じに見えるが別物）。
- SGUB Player の `WP:` 表示は SRAM 256 byte と pool payload の一致で判定するので、繰り返し並べた entry でも正しく ID が出る。
- 実測（2 チャンネル、16 行 × speed 6）: 3WS8PNB の WS effect 1 は 38,722 → 15,566 byte（pool 33 個を 196 回コピー）、effect 5 は 26,818 → 21,980 byte。chipType 0 の出力は変化なし。
- 検証: Furnace の `python test/sgub/v4_check.py build/Release/furnace.exe`（ws_s3w2／ws_3ws8pnb）。`test/sgub/smoke.py` は v1 前提で以前から失敗する。Furnace を開いたままだと `furnace.exe` のリンクが LNK1104 で失敗する。Git Bash から `cmake --build ... -- /m` を渡すと `/m` がパスに変換されて MSBuild がビルドしないので、PowerShell で実行するか `MSYS_NO_PATHCONV=1` を付ける。
