# GUI の 640×480 フレームバッファ、オーバースキャン border、CRT フィルター

- 表示は FPGA（`src32-sv` の `video_timing.sv`: 640×480 active / 800×525 total、`vdp_pcg.sv` は全モードを 640×480 へ `rx>>1`/`ry>>1` で拡大）に合わせる。`src/render.rs` のフレームバッファは常に 640×480（`FRAME_WIDTH/HEIGHT`）で、border color は入れない。border は raster パスで外側の `OVERSCAN`（8 px）にだけ描き、窓へ合わせる単位は `DISPLAY_WIDTH/HEIGHT`（656×496）。旧 `PRESENT_*`/`BORDER_SIZE` は廃止した。
- 描画経路: CPU で 640×480 sRGB テクスチャへアップロード → raster パス（`Rgba8UnormSrgb` 656×496、border は線形化した uniform）→ CRT 無効なら bilinear で窓へ、有効なら bloom 横・縦（半解像度 `Rgba16Float`）→ CRT 合成。サーフェスが sRGB でないときは CRT シェーダー内で sRGB へ符号化する。
- スキャンラインはゲストの 1 ラインに 1 本。ピッチは `source_pixel_pitch(VDP フレームの幅, 高さ)` で、320×240 は (2,2)、640×240 は (1,2)、640×480 は (1,1)。ピッチを 640×480 などに固定すると 240 ラインのモードでスキャンラインが 2 倍の本数になるので、フレームごとに `fb.dimensions()` を渡す。オーバースキャン領域も同じ格子で走査する（border にもスキャンラインが出る）。
- 1 ラインの出力画素数 `line_px` が 1.5〜3.0 の間でビームを `smoothstep` で弱める。480 ラインのモードを 2 倍程度の窓で見ると平坦に近くなるのは仕様（モアレ回避）。ビーム無効時は `smoothstep(0.3,0.7,f)` で隣のラインと混ぜる。最近傍のラインを選ぶだけにすると、曲面の縁が階段状になる。
- ビーム幅の既定は sigma 0.18（暗）〜0.33（明）ライン。0.45 程度まで広げると白でスキャンラインがほぼ消える。
- 曲面を使うとマウス座標がずれるので、`mouse_position_to_guest` に `render::crt_warp`（シェーダーの `warp` と同じ式）を通す。式を変えるときは両方そろえる。曲率 0 のときは warp を通らず従来の式のまま（境界値テストの丸めを変えない）。
- 設定: `config.yaml` の `display.crt`（bool か mapping。未知キーと範囲外はエラー）、`--crt`/`--no-crt`、`--debug-gui` の「Display」メニュー。GUI 専用で testbench には影響しない。
- 検証: `cargo test --release --bin cpt32`（render の CRT 設定・ピッチ・拡大・warp のテストとマウスの曲面テスト）。`config.yaml` に `bios` があるとき、`package_*` の 2 件は変更前から失敗する。見た目は、ヘッドレスの wgpu で raster と CRT のパスを PPM に描く一時テストで 3 モードとも確認した（コミットしていない）。実ウィンドウのスクリーンショットでは確認していない。
