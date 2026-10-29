# Tang Console の Foundation コンソールを VDP 表示

- `foundation/boot/build.py --platform tang-console` の既定をPCG表示＋UART入出力へ変更。旧VDPなしRTLには `--console uart` を指定する。生成manifestに `console` を記録する。
- `FND_CON_TANG` は既存PCG/ANSI/スクロールドライバを使うが、PeC入力ポーリングとワイドフォントインストールを無効にする。UART入力は従来どおり。日本語などは `?` に代替する。
- UARTブートにはBIOSのフォント設定がないため、Tang専用 `con_tang_init` がUNSCIIの8×8両バンク、同じ行を倍化した8×16バンクと先頭64色RGB222を初期化する。通常エミュレータ構成のフォント保持契約は変更しない。
- consoleのgrantにはVDP、テキスト・属性、CLUT `0x10012C00..0x10012CBF`、小フォント `0x10020000..0x10021FFF`、UARTが必要。存在しないPeC入力とワイドフォント領域は許可しない。
- ホスト検証: `python -m unittest discover -s foundation/tests/console -p test_tang.py -v`、`test_headless.py`、`test_console_host.py` が通過。一時ディレクトリはWindowsの権限問題を避けるためworkspaceの `target/tmp` をTEMP/TMPに使った。
- 実機用: `python foundation/boot/build.py target/foundation-vdp --platform tang-console --clang "C:/Program Files/LLVM/bin/clang.exe" -j 6`。隣のsrc32-sv `scripts/uart_load.py COM7 --foundation ../foundation-fpga/target/foundation-vdp/fpga-boot.json` でロードする（RAMディスク内容を置換）。DTR/RTSはlow。
- 実機のOS/shell起動、UARTコマンド、16MiB RAM、FS2の書込・読出・コピー、3表示モード切替を確認。FPGA bitstreamはsrc32-svの `build/vdp-bitmap-p2/.../soc_top.fs` を使用しRTL再ビルド不要。
- エミュレータをゴールデン基準とする。隣のsrc32-sv `scripts/check_foundation_vdp_golden.py` は実ドライバの3モードのVRAM/レジスタをRTLと現行エミュレータVDPへ再生し、921,600画素と15レジスタ読出しが完全一致。全OSブートをエミュレータで比較したテストではなくコンソールドライバの静止画比較（blink無効）。キャプチャーボード画像は色変換・圧縮・拡縮があるので厳密比較に使わない。
