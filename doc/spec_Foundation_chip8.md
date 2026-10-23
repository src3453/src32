# Foundation CHIP-8

`chip8 [-c N|--cycles N] ROM.ch8` はオリジナル CHIP-8 向けの外部コマンドです。
表示は `/dev/sgc`（画素は SGC の矩形描画）と `/dev/fb0`（表示有効化とCLUT）、音声は `/dev/sgu`、物理キーボードは `CON_KEY_STATE` を利用します。
ROM は Foundation のファイルパスで渡します。空ファイル・3584 bytes を超える ROM は拒否します。

## 互換性

4 KiB RAM、16個の8-bitレジスタ、16-bit I、12段のスタック、ロード先 `0x200`、64×32画面。
CHIP-48、SUPER-CHIP、XO-CHIP の拡張命令や互換モードは提供しません。
COSMAC 1802 本体や VIP ROM をエミュレートしないため、`0NNN` のネイティブルーチン呼び出しはエラーで終了します。
不正命令、スタックの過不足、範囲外アクセスは FORMAT で終了し、ホストメモリにアクセスしません。

- `8XY6/E` は VY をシフトして VX へ格納。VF は元の下位／上位 bit。
- `8XY1/2/3` は VF を0にし、`FX55/65` は I を X+1 進めます。
- 算術命令は入力 VX/VY を先に読み、結果の VX、最後に VF を書きます。
- `BNNN` は V0 を足します。`FX1E` は VF を変えません。
- DXYN の開始座標は modulo 64/32、はみ出す画素はクリップ。XOR と衝突 VF を実装します。
- DXYN は60 Hz の描画許可を待ち、1回の許可で1命令だけ描画します。
- `FX0A` は押したキーの解放を待って値を返します。待機中もタイマーを更新します。
- delay/sound は Foundation の単調 tick から分数を積算して60 Hzで減算。
  sound に1だけ設定した場合は無音、2以上なら0に到達するまで SGU の440 Hz矩形波を鳴らします。

引数省略時の速度は従来どおり上限700命令/秒です。
`-c N` / `--cycles N` は 1 VSync（60 Hz換算）あたりのサイクル数を指定します。
ここで1サイクルは CHIP-8 の1命令で、SRC32 / COSMAC 1802 のハードウェアサイクルではありません。
N は十進整数1〜10000。例: `chip8 -c 20 /demo.ch8` は上限1200命令/秒です。
オプションは ROM パスの前後に置けます。`--` 以降はオプションとして解釈しません。
範囲外、数値でない値、値の省略、重複、不明オプション、ROMの過不足は使用法を表示して BAD_ARG で終了します。
引数検証は ROM やデバイスを開く前に行います。

指定時は Foundation の単調 tick から60 Hz境界を積算し、各境界でN命令分の予算を与えます。
実際のホスト VSync IRQ を待つ仕組みではありません。
待機に使った予算は捨て、delay/sound タイマーの60 Hzは指定値にかかわらず維持します。
100 Hz scheduler 上のタイマー／描画時刻には10 ms単位の量子化があります。
描画や IPC の負荷で処理が遅れる場合があり、COSMAC VIP の cycle 単位のタイミング再現ではありません。
停止後の CPU 追い付きは250 ms分までに制限し、タイマーは経過時間分を進めます。
乱数は起動 tick を種にしたローカル xorshift32。再現性や暗号用途の保証はありません。

## 表示・操作・終了

白黒（CLUT 0 黒・1 白）で5倍表示（320×160）、上下に40 pixelsずつ黒帯を設けます。
全面を `SCREEN_FILL` で黒にした後、前回表示から変化した画素だけを横方向の連続区間ごとに `RECT`（5×5 pixel 単位）で SGC へ送ります。SGC の描画面はフレームをまたいで保持されるため、毎回76800 byteを書く必要がありません。FIFO（256 word）が満杯のときは残りを次のループで送り、表示は少し遅れます。
PCG 出力と競合しないよう、描画中にコンソールへ文字は書きません。
音声は SGU 0 channel 0、256 samples の矩形波、左右同音量。
device の排他取得に失敗するとエラーで終了します。

| キーボード | CHIP-8 キーパッド |
|---|---|
| `1 2 3 4` | `1 2 3 C` |
| `Q W E R` | `4 5 6 D` |
| `A S D F` | `7 8 9 E` |
| `Z X C V` | `A 0 B F` |

Shift/CapsLock に関係なく物理キーで入力します。長押しと同時押しに対応します。
Esc で終了し、音声を停止してコンソールを復元します。
`CON_KEY_STATE` の遷移は取得間隔内の順序を保持しないため、極端に速い複数回の押し直しは区別できません。

## ビルドと起動

`foundation/boot/build.py` / `foundation/run.py` の通常ビルドに `/bin/chip8.wax` が含まれます。
自作サンプル ROM の生成と配置:

```powershell
python examples/foundation-chip8/demo.py target/chip8-root/demo.ch8
python foundation/run.py --root target/chip8-root
```

Foundation のシェルで `chip8 /demo.ch8` を実行します。
速度を指定する例は `chip8 --cycles 20 /demo.ch8` です。
任意のキーでそのキーの16進数字を表示して音を鳴らします。Esc で戻ります。
他の ROM は `target/chip8-root` 配下に配置して同様にビルドできます。
ROM の再配布・ライセンスは各 ROM の条件に従ってください。このサンプルは本リポジトリ用に作成したものです。

## 検証

```powershell
cargo build --release --bin src32_testbench
python -m unittest discover -s foundation/tests/chip8 -v
$env:FND_MM='slot'
python -m unittest discover -s foundation/tests/chip8 -v
Remove-Item Env:FND_MM
```

ホストテストは WAX と同じ C コアを実行します。ゲストテストはシェルからの起動、
fb 画素、SGU の周波数・音量・波形、両コア全チャンネルと PCMRAM、
押下・長押し・同時押し・解放、終了/fault時の解放を確認します。
testbench の `--key FRAME:USAGE:0|1` で物理キー遷移を注入し、
`--expect-final-mem-u8 ADDRESS=VALUE` で終了時のメモリ／レジスタを確認します。
これはスピーカーからの出音や聴感品質の確認とは別です。

描画の回帰テストは VRAM に加え `--expect-pixel X:Y=0xRRGGBB` で VDP 合成後の表示色を検査します。
`--dump-framebuffer PATH.ppm` で表示画像を PPM 形式に保存できます。
外部 ROM は `CHIP8_TEST_ROM` にホスト上のパスを設定すると、再配布せず一時ディスクに置いて表示を検証できます。
`CHIP8_TEST_CAPTURE` を指定するとそのテストの表示画像を保存します。

実装の参照:
[Mikolay の original CHIP-8 reference](https://github.com/mattmikolay/chip-8/wiki/CHIP%E2%80%908-Technical-Reference)、
[命令表](https://github.com/mattmikolay/chip-8/wiki/CHIP%E2%80%908-Instruction-Set)、
[RCA COSMAC VIP manual](https://www.bitsavers.org/components/rca/cosmac/COSMAC_VIP_Instruction_Manual_1978.pdf)。
