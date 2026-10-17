# CPT32

SRC32 CPU をベースにしたレトロ・モダン系ファンタジーコンソールと、その Rust 製エミュレータです。CPU と MMIO バスを中心に、VDP による画面合成、VPU による 3D 描画、SGC による 2D 描画、SGU による音声、DMA・割り込み・周辺機器などを実装しています。仕様と実装の詳細は [ドキュメント一覧](#ドキュメント) を参照してください。
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/src3453/src32)
## 主な内容

- **CPT32 エミュレータ** — Rust で実装した CPU、メモリ／バス、描画・音声・周辺機器。
- **SRC32** — 32-bit RISC CPU。命令セット仕様は [`doc/spec_CPU.md`](doc/spec_CPU.md)。
- **sol** — スタック指向言語。Python 製 VM と SRC32 アセンブリへのコンパイラを含みます。標準ライブラリ（`sol/solstd`、`sol/cpt32`）は名前空間を宣言しており、`pcg.prnscr` や `softfloat.sf32_add` のように `名前空間.名前` で参照します（[`doc/spec_sol.md`](doc/spec_sol.md)）。
- **開発サンプル** — [`3d_cube.sol`](3d_cube.sol)、[`mouse_cursor.sol`](mouse_cursor.sol)、[`vpu_tnl_triangle.sol`](vpu_tnl_triangle.sol) など。
- **CPT32 DOS** — DSFS／フロッピーを操作するシェル（[`dos.sol`](dos.sol)）。

## ビルドと実行

Rust のツールチェーンが必要です。まずリリースビルドします。

```sh
cargo build --release
```

エミュレータはゲストプログラムのバイナリを指定して起動します。

```sh
cargo run --release --bin cpt32 -- path/to/program.bin
```

`sol` のサンプルをコンパイルして起動する例:

```sh
python3 tools/solc/solc.py compile 3d_cube.sol -o /tmp/3d_cube.asm
python3 tools/asm/asm.py /tmp/3d_cube.asm -o /tmp/3d_cube.bin
cargo run --release --bin cpt32 -- /tmp/3d_cube.bin
```

Assembler は出力拡張子で形式を選択します。raw `.bin` が既定で、`-o` の出力先を `.wax` にすると WAX v2 を直接生成します。先頭に `.ORG` がある場合はその番地、それ以外は 0 が WAX base になり、先頭 padding と forward `.ORG` 間の hole はファイルに格納されません。各領域は独立した block としてロードされ、hole には書き込みません。entry は既定で最初の byte、ラベルまたは絶対番地で指定できます。

```sh
python3 tools/asm/asm.py program.asm -o program.wax --entry start
```

デバッグ GUI を有効にするには `--debug-gui` を指定します。デバッグ GUI 起動時に一時停止する場合は `--start-paused` も指定してください。
デバッグ GUI の `Windows` メニューから `Bus Composition View` を選ぶと、領域サイズをバーで比較し、その下の一覧で登録デバイスの開始・終了アドレスと容量を確認できます。未登録のアドレス範囲は一覧に表示されません。
`Performance` ウィンドウではCPU使用率のプログレスバーと履歴グラフ、命令実行速度、IPC（instructions per cycle）を確認できます。使用率の最大値は `Debug Controls` の `Cycles / frame` 設定に連動します。
`Debug Controls` の `Reset` はCPUをアドレス `0x00000000` から再初期化し、IRQコントローラの保留要求・許可・入力レベルを消去して一時停止します。ゲストRAMの内容は保持します。

```sh
cargo run --release --bin cpt32 -- path/to/program.bin --debug-gui --start-paused
```

エミュレーターは既定でパイプライン CPU（Extension p）で起動します。参照用のスカラー CPU は `--cpu scalar`、Cached Interpreter は `--cpu cached` で選択します。`--cpu pipeline` でパイプラインを明示できます。`config.yaml` の `cpu.microarchitecture` でも指定でき、CLI が優先されます。

Cached Interpreter は RAM 上の命令を直接マップ型のデコードキャッシュに保持します。RAM の該当 4 KiB ページが書き換わるとキャッシュを無効化します。キャッシュヒット時は命令フェッチのバスアクセスを省略します。通常命令は1サイクル、MUL/MULHは3サイクル、DIV/DIVU/MODは16サイクルです。既定 CPU は変更していません。

3種類の CPU を同じループワークロードで比較するには、リリースビルドでベンチマークを実行します。位置引数は順にループ反復数と測定サンプル数です（既定値: `10000`, `5`）。実行ごとにレジスター・PC・データメモリー・完了命令数を照合し、差異があれば失敗します。

```sh
cargo run --release --bin cpu_bench -- 10000 5
```

ワークロードは繰り返し実行される依存 ALU 演算、MUL とその直後の利用、ストア／ロード、load-use 依存、ループ分岐を含みます。出力は各モードの中央値、命令キャッシュ hit/miss、Cached の Scalar／Pipeline に対する speedup を示します。wall-clock 性能はホスト環境依存です。CPU サイクル数と CPI はゲスト側のカウンターです。

ログは `tracing` から標準エラーへ出力され、端末ではレベルごとに色分けされます。既定のレベルは `info` です。`config.yaml` の `logging.level` で `trace` / `debug` / `info` / `warn` / `error` / `off` を指定できます。`RUST_LOG` が設定されている場合はそちらを優先します。

```sh
RUST_LOG=debug cargo run --release --bin cpt32 -- path/to/program.bin
```

PCG の raw font file は引数ではなく、起動時の `config.yaml` で指定します。`font.bin` は PCG font bank 0/1 用の2 KiBまたは4 KiBファイルです。相対 path は `config.yaml` のある directory を基準にします。設定を省略した場合は内蔵フォントを使用します。

```yaml
pcg:
  font: "assets/font.bin"
```

旧来の `<program.bin> [font.bin]` 形式は使えません。

CPU の対話型モニターは次のように起動します。

```sh
cargo run --release --bin cpt32 -- monitor [program.bin] [--cpu scalar|cached|pipeline]
```

## CPT32 DOS（DSFS シェル）

`dos.sol` は DSFS ボリュームを操作する小さなシェルです。文字は PCG コンソールに表示し、キーボードは PeC の入力 FIFO を IRQC source 3 で待って読みます。ファイルはドライブ 0／1 の DSFS ボリューム（`doc/spec_DSFS.md`）として扱われ、`FORMAT` はゲスト自身が superblock・bitmap・directory を書きます。

```sh
python3 tools/solc/solc.py compile dos.sol -o dos.sol.a
python3 tools/asm/asm.py dos.sol.a -o dos.bin
```

ボリュームは `tools/dsfs/dsfs.py` で用意し、スロットに接続して起動します。`config.yaml` に `bios` があると BIOS が起動して RAM のプログラムは実行されないため、`src32_testbench` で直接動かす場合は `bios` のない directory から実行します。

```sh
python3 tools/dsfs/dsfs.py mkfs disk0.img --sectors 2880
python3 tools/dsfs/dsfs.py put disk0.img HELLO.TXT hello.txt
cd path/to/config-dir && cargo run --release --bin src32_testbench -- /path/to/dos.bin
```

| コマンド | 内容 |
|---|---|
| `DIR` | ファイル一覧（サイズとブロック数） |
| `TYPE NAME` | テキストとして表示 |
| `NEW NAME` / `REN OLD NEW` / `DEL NAME` | 作成・名前変更（時刻は保持）・削除 |
| `COPY SRC DST` | ドライブ間コピー（`COPY 0:A.TXT 1:A.TXT`） |
| `EDIT NAME` | ラインエディタ（`L` 一覧、`<n> テキスト` 置換、`I n テキスト` 挿入、`D n` 削除、`A テキスト` 追加、`W` 保存、`Q` 終了） |
| `HEX NAME` | HEX エディタ（`N`/`P` ページ、`G` 移動、`E` 書き換え、`W` 保存、`Q` 終了） |
| `FORMAT [N]` | ドライブ N を DSFS でフォーマット |
| `DRV N` / `FREE` / `CLS` / `HELP` / `EXIT` | ドライブ選択・空き容量・画面消去・ヘルプ・終了 |
| `MODE 40` / `MODE 80` | 表示幅を切り替えて画面を消去 |

プロンプトは `0> ` の形式です。

名前は 8.3 形式（大文字、`N:NAME.EXT` でドライブ指定）で、ファイルは 1 個 64 KiB までです。回帰テストは `tests/dos_smoke.rs`（`cargo test --test dos_smoke`、`dos.bin` のローカルビルドが必要）です。

## SGU Music Editor

ROM・CPU・VDP を起動せず、独立した ImGui エディタで 16 チャンネルの曲を編集します。

```sh
cargo run --bin music_editor
cargo run --bin music_editor -- path/to/song.yaml
```

上部に再生／停止・テンポ・保存／Export、中央に order と instrument、下部にテキスト形式の pattern を配置しています。
各チャンネルは **NOTE / INS / VOL / FX** の独立したカーソル位置を持ちます。横スクロールはカーソルに追従します。
Furnace のドキュメントにあるトラッカーの操作概念を参考にした独自実装で、Furnace のコードや GUI コンポーネントは使用していません。

| 操作 | キー |
|---|---|
| フィールド／行移動 | 左右／上下矢印 |
| 次／前チャンネル | Tab / Shift+Tab |
| 先頭／末尾行、16行移動 | Home / End、PageUp / PageDown |
| 次の1行へ移動 | Enter |
| C〜B のノート入力 | `Z S X D C V G B H N J M` |
| 次 octave のノート入力 | `Q 2 W 3 E R 5 T 6 Y 7 U` |
| INS / VOL / FX の値 | `0`〜`9`、`A`〜`F` を2桁入力 |
| FX の種類 | `P` = pan、`T` = tempo、`V` = SetVolume |
| 選択フィールドだけ消去／ノート Off | Delete / NOTE 上の Backspace |
| 入力途中のキャンセル | Escape |
| 再生／停止、pattern にフォーカス | F5 / F8、F6 |
| octave／編集ステップ変更 | `[` / `]`、Ctrl+`[` / Ctrl+`]` |
| 保存／Save As／読込 | Ctrl+S / Ctrl+Shift+S / Ctrl+O |

ノートと完成した16進値の入力後は編集ステップ分だけ行を進めます。ステップ0は同じ行に留まります。ノート入力時は選択中の instrument で約0.27秒の試聴を行います。Note Off は試聴しません。
FX tempo は `1E`〜`96`（30〜150 BPM）。VOL は独立した音量指定なので pan／tempo と同時に設定できます。
同じ cell に VOL と FX `V` がある場合は VOL を優先します。instrument macro に値のある lane は cell の音量／pan より優先します。

### 楽器と保存

- Wavetable は 256 個の unsigned 8-bit sample。グラフ上でクリック／ドラッグして描画し、専用 **Wavetable Editor** で sample 値、sine／triangle／saw／square、gain・offset・phase・normalize・invert・reverse・smooth・double・quantize を編集できます。
- PCM は mono／stereo の uncompressed 16-bit PCM WAV。相対 path は project の親 directory が基準です。Save As では参照先を保って path を書き直します。sample rate は 1〜2,097,120 Hz、全資産の合計は 1 MiB 以下。
- Noise と、音量／半音 pitch／pan の最大256 step macro をグラフで編集できます。左クリック／ドラッグで値を描画して step を選択し、右クリックでその step の値を消して直前の値を保持します。数値入力と loop 設定も使えます。macro は VSYNC ごとに進み、音量／pan の初期値は instrument 設定、pitch の初期値は0です。
- 保存形式は YAML schema v1。`format_version`, `tempo_bpm`, `ticks_per_row`, `repeat`, `orders`, `patterns`, `instruments` を持ちます。pattern はそれぞれ1〜256行×16 channelで、YAML の `rows` 配列の長さが有効な行数です。旧 project は従来どおり64行で読み込み、新規 pattern も64行です。Pattern の Rows を短くすると末尾行を削除し、長くすると空行を追加します。ID は0始まりです。
- 欠損／不正 PCM の曲も保存できますが、preview／Export はエラーになります。Export は `.sgub` と同じ directory の汎用 `sgu_music_driver.sol` を出力します。既存ファイルの置換は確認後、両方の temporary file が書き込めてから実行します。

### SGUB v1 とゲスト再生

SGUB header は20 byte: `SGUB`, version=1, voices=16, flags（bit0 repeat）, reserved=0、big-endian u32 の frame count / initial record count / total size。
initial record の後に各 frame の big-endian u16 record count と record を格納します。
record は target u8 / address u24 big-endian / value u8 の5 byte。target 0/1 は各 SGU の offset `0..8FF`、target 2 は共有 PCMRAM の offset `0..FFFFF`。
不正 header、範囲外 address、切り詰め、余剰 byte は再生前に拒否します。VSYNC は60 Hz、tracker tick は accumulator に BPM を加え150ごとに発生します。

```sh
python3 tools/solc/solc.py compile sol/cpt32/sgu_music_driver.sol --out target/sgu_music_driver.asm
python3 tools/asm/asm.py target/sgu_music_driver.asm -o target/sgu_music_driver.bin
cargo run -- target/sgu_music_driver.bin --music-stream path/to/song.sgub
cargo run --bin src32_testbench -- target/sgu_music_driver.bin --allow-running --music-stream path/to/song.sgub --frames 3 --expect-mem-u8 0x80020800=0x01 --expect-mem-u8 0x80020801=0xB8
```

Windows で Python が `python3` として見つからない場合は `py -3` を使用します。
stream は主 RAM の `0x00200000` にロードされます。driver image は2 MiB未満、stream は `0x00E00000` byte 以下。
driver は IRQ0 ごとに1 frame を処理し、repeat／非repeat終端停止を扱います。`--expect-mem-u8` は複数指定でき、各 frame の IRQ0 処理後に全指定値を検査します。

## ドキュメント

仕様・ガイド・設計資料は、Obsidian 互換の Wiki 形式で整理した [日本語ドキュメントホーム](doc/ja/Home.md) または [Documentation Home](doc/Home.md) から参照できます。

- [システムと CPU](doc/Systems.md)
- [ハードウェア仕様](doc/Hardware.md)
- [言語・パッケージ・ツール](doc/Software.md)
- [チュートリアルと開発ガイド](doc/Guides.md)

英語原文があるページには `doc/ja/` 以下に対応する日本語版があります。
