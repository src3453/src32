# VDP (Video Display Processor) 仕様

## 1. 目的と責務

VDPは画面タイミングと最終ピクセル合成を担当する。GP (Graphics Plane) はVRAM上のフレームバッファではなく、描画エンジンの出力先を識別する8個の論理レイヤー `GP0`～`GP7` である。GP番号が小さいほど奥、番号が大きいほど手前として固定する。ゲーム固有の「背景」「UI」等の意味は割り当てない。

ビットマップ描画、PCG、SGC、VPU等は、それぞれ透明度付きピクセルを1つ以上のGPへ供給する。各GP内では供給元を合成し、VDPはGP0からGP7の順に合成して表示出力を作る。VDPは供給元の内部属性形式を解釈しない。

```text
Bitmap / PCG ─┐
SGC sprites/2D ─┼─> GP0..GP7 (transparent RGBA pixel streams)
VPU output ───┘             │
                    VDP compositor
                             │
                       display output
```

## 2. 合成モデル

- 内部作業画素はRGBA8888、straight alphaとする。透明はA=0、完全不透明はA=255。
- 各供給元はGPごとに1画素を出力し、同じGP内の供給元は登録順にsource-over合成する。後から合成した供給元が前面となる。
- VDPはGP0からGP7へsource-over合成する。高いGP番号が低いGP番号を覆う。
- source-overは `out.rgb = (src.rgb * src.a + dst.rgb * (255-src.a) + 127) / 255`、`out.a = src.a + (dst.a * (255-src.a) + 127) / 255` とする。整数中間値は十分な幅を持つ。
- GP/供給元が無効なら透明画素を出力する。画面外画素も透明とする。
- Border colorは全GPの後ろに置く不透明な背景色である。
- 表示無効時は全画素を黒で出力する。

### 2.1 PCG

PCGは独立した表示モードではなく描画供給元となる。初期実装では1つの設定可能GPへ供給し、文字セルの背景色・前景色を不透明画素として出す。背景色を透明にする設定は後方互換用でなく新規機能として別途定義する。`SCREEN_MODE`、フォントバンク、カーソルはPCGの属性であり、最終画面モードを排他的に切り替えない。

`SCREEN_MODE` (`0xF003`) は次の値を取る。`0`～`2`以外の書込みでは現在値を維持し、VDP error status bitを立てる。

| 値 | セル構成 | 解像度 |
|---:|---|---:|
| `0` | 40列×30行、8×8 glyph | 320×240 |
| `1` | 80列×30行、8×8 glyph | 640×240 |
| `2` | 20列×15行、16×16 BMP glyph | 320×240 |

mode 2の文字セルは `row * 20 + column` 順で、VRAM `0x00000`からUTF-16BEのBMP code unitを2 byte（上位byte先行）で格納する。属性は既存のFG base `0x01000`とBG base `0x02000`を使い、同じcell indexの各1 byteの下位6 bitをCLUT indexとする。色swap、前景/背景とも不透明な描画、カーソルのblinkと色反転は既存PCG動作に従う。`PCG_FONT_BANK`はmode 2で無視する。

mode 2のカーソル範囲はX=0～19、Y=0～14。8-bit `CURSOR_LINES`の各bit（MSBがglyph row 0）は隣接する2 scanlineへ適用する。UTF-16 surrogate code unitはU+FFFDとして表示し、surrogate pairを使った補助平面文字表示は行わない。

### 2.2 ビットマップGP

既存のGP0 indexed bitmapとCLUTはGP供給元の初期実装として維持する。256色パレットモードではインデックス0～255を不透明なCLUT色へ変換する。透明色インデックスの選択機能は拡張レジスタとして定義するまで有効にしない。

ビットマップの色形式は共通レジスタ `0x04` (`BITMAP_COLOR_MODE`) で選択する。reset値は256色パレット。形式は次の通り。

| 値 | 形式 | VRAM上の画素データ | 画素あたり |
| --- | --- | --- | --- |
| `0` | 256色パレット | `0x00000`～、1 byteのインデックス | 8 bit |
| `1` | RGB555 | `0x100000`～、big-endian 16-bit word | 16 bit |
| `2` | RGB888 | `0x200000`～、R, G, Bの順に3 byte | 24 bit |

各ビットマップは320×240で、行末パディングはない。RGB555はbit 15を予約ビットとし、VDPは無視する。bit 14～10をR、9～5をG、4～0をBとして、それぞれ5-bit値を8-bitへ複製拡張する。256色パレットのCLUTはVRAM `0x12C00`～`0x12EFF` に256エントリをRGB888 (R, G, Bの3 byte) で格納する。先頭64色は既存CLUT配置を維持する。別領域に画像を置くため、各形式の切替でCLUT、PCG、SGC用の共有データを上書きしない。

## 3. 画面とタイミング

初版の表示領域は320×240。描画供給元の幅・高さが異なる場合、VDPは自動拡大縮小しない。供給元側で座標変換する。既存の表示タイミング、border、framebuffer APIは維持し、合成済みRGB画素を返す。

## 4. GPと供給元の接続

各供給元は `OUTPUT_GP` を持ち、0～7を指定する。予約値は無効設定としてエラーステータスを立て、最後の有効値を保つ。供給元はGP番号と画素ストリームだけでVDPへ接続し、VDPはSC/VPU固有の優先度や属性を知らない。

GPレイヤー順はGP番号で決まり、供給元内の順序は各供給元が決める。SCのスプライト優先度はSC内だけで有効であり、GP間の順序を上書きしない。SCをPCGより手前へ置く場合はSCの出力GPをPCGのGPより大きくする。

## 5. メモリとMMIO

- VRAM: `0x10000000`–`0x103FFFFF`、4 MiBを共有する。
- VDP MMIO: `0x80000000`–`0x8000FFFF`。
- 共通レジスタ `0x04` は `BITMAP_COLOR_MODE`。`0`=256色パレット、`1`=RGB555、`2`=RGB888。`0`～`2`以外の書き込みは現在の色形式を維持し、STATUSのerror bitを立てる。
- PCG `SCREEN_MODE=2`ではVRAM `0x00000`～`0x004AF`を文字セルに使う。600セル分を `row * 20 + column` 順に並べ、各セルをUTF-16BE（上位byte先行）の2 byteで格納する。FG/BG属性baseは従来どおり`0x01000`/`0x02000`で、各600 byteの下位6 bitをCLUT indexとする。mode 0/1のフォントバンクとraw 2/4 KiBファイル読込み契約は変更しない。
- PCGとSGC仕様の`GLYPH`コマンドは、読み取り専用のGNU Unifont 18.0.01 Plane 0 ROMを共有する。ROMは`assets/unifont-bmp.chr`として実行ファイルへ埋め込み、CPU可視VRAMには配置しない。BMP code unit順の65,536 slot（各32 byte、合計2 MiB）を持ち、各glyphは行優先の16行×2 byte、左pixelを最上位bitとする。8×16 glyphは左byteに置き右byteを0にし、16×16 glyphは2 byteを使う。未収録code unitとUTF-16 surrogateはU+FFFD glyphへ置換する。入力、変換ツール、ライセンス情報は`assets/unifont-18.0.01.hex.gz`、`tools/unifont_hex_to_chr.py`、`assets/UNIFONT-LICENSE.txt`に記録する。
- 既存VRAMオフセットとPCGフォント/セルデータを直ちに移動しない。GP追加時の割当は各供給元のbase/stride設定と共に後方互換性を確認して定義する。
- 既存の `DISPLAY_MODE=Graphics/PCG` は互換レジスタとして読み書きできるが、新合成経路では供給元の有効/無効とGP割当へ変換する。新仕様でGraphics/PCGを画面全体の排他的モードにはしない。
- 初期移行ではbitmapはGP0、PCGの出力GPはVDPレジスタ `0xF00A`（0～7、reset値0）、PCGの重ね描き有効は `0xF00B`（reset値0）、SCのGPはSC `OUTPUT_GP`（reset値1）、VPUのGPはVPUレジスタ `0x0014`（reset値7）で選択する。Graphics modeでは `PCG_OVERLAY_ENABLE` を立てるとbitmapとPCGを同時に出力できる。旧PCG modeはPCGを画面ベースとして使う互換動作を維持する。
- レジスタの詳細オフセットは実装移行時に定義する。現行の基本レジスタ・PCGレジスタの意味は移行表を併記するまで削除しない。

## 6. 同期とフレーム整合性

VDPは走査位置ごとに全供給元の同一座標を合成する。初版エミュレータはフレーム取得時に各供給元の現状態を読む。表示中に属性/VRAMを書き換えた場合、書き込み後の画素から反映される。VBlankラッチやダブルバッファは初版では要求しない。

## 7. 実装境界

VDPのcompositorは純粋なRGBA合成処理として切り出し、供給元の状態機械/MMIO実装と分離する。SGC/VPUが未実装または無効のときは透明供給として扱い、既存GP0/PCGの画素出力を壊さない。VPUのカラーターゲットは最終画面へ直接混ぜず、設定されたGPの供給元として扱う。
