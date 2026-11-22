# ベアメタル C の Flappy（examples/flappy-c）と DMAC 描画の予算

- GP0 を毎フレーム全面描き直すとき、ちらつかない条件は「`irqc_wait_vsync()` 直後から描き始め、1 フレーム（`CYCLES_PER_FRAME` = 800,000）以内に終える」こと。testbench／GUI は `vdp.tick()`（GP0 取り込み）→ IRQ0 → `cpu.run(800000)` の順なので、はみ出すと描画途中の画面（例: 文字の縁取りだけ、後半の部品が欠ける）がそのまま出る。
- DMAC の `dmac_fill`／`dmac_blit` は 1 回あたり CPU 側で約 1000 サイクル（設定 MMIO とポーリング）かかる。転送自体は 1 beat に read+write の 2 バスサイクルで、8-bit 幅は 2 サイクル/byte、32-bit 幅は 2 サイクル/4 byte。小さな矩形を数百回塗る（5×7 フォントをランごとに 5 パス）と 1 フレームを超える。最初の実装はタイトル 85 万、ゲームオーバー 158 万サイクルだった。
- 対策は「部品を起動時に RAM へ描き、毎フレームは大きな BitBlt を少数回」。source stride 0 で 1 行を縦に繰り返せる（土管）。視差スクロールで 32-bit beat を保つには、1〜3 byte ずらしたコピーを持ち、`off & ~3` を転送元オフセットに使う。色キー付き転送（`DMAC_KEY_EN`）は 8-bit 幅で画素単位に効く。
- 計測は PeC timer 0 を free-running・prescaler 0（48 MHz、`COUNT_LO` = `0x80040408`）にして UART へ出す（`-DFLAPPY_PROFILE`）。`test_every_frame_fits_in_one_vsync` は最大値が半フレーム未満であることを確かめる。
- この環境は Clang 20 と llvmlite（LLVM 22）が不一致だが、この例は -O2 でビルド・実行できた。Ninja がないときは `driver.py` に main.c と `sdk/cpt32/src/{dmac,irqc}.c` を直接渡せばよい。
- 検証: `CLANG=... python -m unittest discover -s examples/flappy-c -v`（Clang と `src32_testbench` が必要。なければ skip）。
