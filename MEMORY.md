## 作業で得た知識・注意点

- WAX v1 (doc/spec_WAX.md): .wax/.wal は同一 64B ヘッダのコンテナで種別のみ違う。ロード先は
  section ごとではなく base ひとつ + entry_offset、debug は NOLOAD の付録。ヘッダ複数バイト項目は
  little-endian。ローダはマジック sniff で素朴 .bin (base 0/PC 0) と両対応し、CPT32PKG manifest は不変。
- `cargo test` (lib) は `src/devices/sgu/sqv4.rs` の `include_str!("../../../../s3w2_core/tests/...")`
  がリポジトリの親ディレクトリ側 `s3w2_core` を指し、存在しないためコンパイル失敗する。wax 単体は
  空スタブを親側に一時配置して `cargo test wax::` で通した。スタブは必ず削除すること。
- testbench の `--expect-mem-u8` で `ST` (32-bit ワード格納) を確認する際は big-endian のため
  LSB は +3 番地に来る (例: 42 を 0x2000 に ST したら 0x2003=42 で確認)。
- BIOS ROM (src/devices/bios.rs): config `bios.image` (config 基準の相対 path) を
  `0xFF000000` の 64 KiB read-only window に zero-pad して map し、entry を返す。
  raw は先頭配置・PC=`0xFF000000`、WAX は extent が window 内のものだけ受付。
  ROM への guest write は silent-ignore (CPU は device 側 bus_error を見ないため fault 化しない)。
  section 欠落・null・config 不在は `Ok(None)` で RAM 起動に fallback。起動経路は
  main raw guest / monitor / testbench の全てで BIOS entry を優先する。
- BIOS POST が一瞬で消える件: GUI も testbench も 1 ホストフレーム毎に IRQ0 パルス→
  `cpu.run(CYCLES_PER_FRAME=800k)`→描画なので、待ち無しの BIOS は最初の 1 フレーム内で
  POST+IPL が完結し POST が描画されない。対策は BIOS 側で VBlank(IRQ0)待ちを入れること
  (sol/bios.sol v0.3: `fn irq0` で ack+計数し `while halt ... end` で 120 フレーム保持)。
  CPU サイクル数のビジーウェイトではホストフレームを跨げない。ハンドラは PENDING bit0 を
  必ず ack すること (ack 無しだと IRQC 出力が High のまま edge が来ず HALT から二度と起きない)。
  IPL直前に `0 IRQC_ENABLE sth` + PENDING クリアを忘れずに (ベクタは BIOS dispatcher を指した
  ままなので)。GUI と testbench は共に毎フレーム IRQ0 を供給するため HALT 待ちは両方で動作する。
  assets/bios.rom は sol/bios.sol ヘッダのコマンドで再構築する (64KiB 上限、`--strip` 必須)。
- デバッグ GUI (`--debug-gui`) のメインメニューバー重なり対策: `src/render.rs` の
  `game_viewport(size, top_reserve_px)` が描画とマウス変換の共通計算を持つ。
  `DebugGui::render_frame` は `ui.frame_height()` (論理 px) を保持し
  `display_framebuffer_scale` で物理 px 化した値を `render_with_overlay` の予約量に渡す。
  ゲーム画面は予約量の下に中央寄せし、通常 GUI (`render()`) は予約 0 で従来通り。
  マウス絶対・相対も同じ予約量で換算する。初フレームは高さ 0 のため 1 フレームだけ
  被る可能性がある。
