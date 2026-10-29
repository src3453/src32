# SGC 円・楕円コマンド

- SGC1に0x15 ELLIPSE、0x16 FILL_ELLIPSEを追加。5語で中心X/Y（signed16）と半径X/Y（0..32767）。各payload上位16 bitは0。radius bit15やheader予約bit違反はFIFO_ERROR、5語消費後に続行する。
- 正の半径では整数sampleの `dx²*ry² + dy²*rx² <= rx²*ry²` を内部とする。輪郭は内部で4近傍のいずれかが外側の画素。clip前の近傍を調べるので画面端に人工的な輪郭を描かない。0半径は線分/点。AAやfloatの近似へ独自に変えない。
- `src/devices/sgc/sgc.rs` が基準実装。FPGAは隣接 `src32-sv/soc/sgc_ellipse.sv` の共有32×32乗算器・64-bit F・48-bit差分で同じ画素を作る。ソフトの直接i64評価とハードの差分加算は別実装として比較する。
- libsgcの `cpt_sgc_ellipse` / `cpt_sgc_fill_ellipse` は既存exports末尾へ追加。既存slot順を変えない。中心clamp/色coalescing/flushは共通、半径の範囲外はBAD_ARGでqueueしない。driverはraw FIFOのままでIPC ABI変更なし。
- focused check: `cargo test --offline --lib devices::sgc::sgc::tests`（10件通過）。`HOST_CC`をClang絶対パスへ設定して `python -m unittest discover -s foundation/tests/libsgc -k HostTests -v`（host通過）。Foundation guest WAL end-to-endは今回未実行。
- FPGA比較は隣接 `src32-sv/scripts/run_sgc.py`。104 frame / 7,987,200 RGBA画素 / MMIO read 940 byte一致。HDMI前307,200画素も一致。source hashは `src32-sv/build/sgc/result.json`。
- 実機はTang Console 138K、楕円入り診断のUART `(31265,0x083ada6c,125060,25200000)` とUSB映像を確認。詳細とRTL化の差は `src32-sv/doc/sgc-ellipse.md`。本番SoC/DDR3/GP統合を完了した意味ではない。
