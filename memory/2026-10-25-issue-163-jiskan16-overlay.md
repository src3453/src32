# 共通 BMP フォントの Jiskan16 差し替え（#163）

- `assets/unifont-bmp.chr` は Unifont 18.0.01 を土台に、X.Org font-jis-misc 1.0.4 の Jiskan16 全6,877字形を上書きした2 MiB ROM。対象は漢字・かなだけでなく JIS の記号・全角英数字・ギリシャ文字等も含む。対象外のスロット・U+FFFD・surrogate fallback は従来どおり。
- Jiskan16 の `ENCODING` は Unicode ではなく JIS X 0208 の2 byte位置。両byteに0x80を加え `euc_jp` で復号する。CP932 を使うと波ダッシュ・マイナスなどの Unicode 対応が変わる。JIS X 0208:1990 で追加された字形など、この1983版BDFにない字形は Unifont を保持する。
- BDF の各字形は `BBX 16 16 0 -2`、行順の16×2 byte。フォント全体の `FONTBOUNDINGBOX` と混同して垂直シフトしない。PCG mode 2 と SGC `GLYPH` はこの ROM を共有し、Foundation `boot/wide_font.py` も同じ ROM から全角フォントを生成する。
- Jiskan16 はパブリックドメイン。上流BDFと COPYING は未変更で同梱し、`.gitattributes` の `-text` で取得時のバイト列を維持する。出典・SHA-256 は `assets/JISKAN16-LICENSE.txt`。Unifont との混合ROMは OFL-1.1 で配布し、元の Unifont HEX は変更しない。
- 再生成: `python tools/unifont_hex_to_chr.py assets/unifont-18.0.01.hex.gz assets/unifont-bmp.chr`。`--check` で一致確認。既定で同梱 Jiskan16 を使う。Python API の `convert(source, None)` は比較用の Unifont 単体ROM。
- 字形変更では座標固定の描画テストも更新する。「あ」の row 1 は `0x0200` なので点灯pixelは x=6。scroll (319,239) なら画面 (7,2)。この座標は wrap と cursor の FG/BG 交換の検証意図を保つために変更した。
- 検証済み: `python -m unittest discover -s tools -p test_unifont_hex_to_chr.py -v`（4件、全65,536スロット比較・Foundation生成C字形・不正BDF）、ROM `--check`、`python scripts/generate_license_notices.py --check`。
- 検証済み: `cargo test --test vdp_pcg_scroll --test vdp_pcg_wide --offline`（3件）、`cargo test --lib devices::vdp::vdp::tests::pcg_mode2 --offline`（5件）、`cargo test --lib devices::sgc::sgc::tests --offline`（8件）。
- 検証済み: `python -m unittest discover -s foundation/tests/console -v`（ホスト・VMゲスト計10件）、`FND_MM=slot` で `python -m unittest discover -s foundation/tests/console -p test_console_guest.py -k wide_characters -v`（1件）。Windows では TEMP/TMP をワークスペース内 `.tmp` に設定した。
- ROM はエミュレータ実行ファイルへ埋め込まれる。既存のエミュレータを再ビルドし、Foundation の既存イメージも新しい字形を使うには再生成する。
