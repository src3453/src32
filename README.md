# CPT32

SRC32 CPU をベースにしたレトロ・モダン系ファンタジーコンソールと、その Rust 製エミュレータです。CPU と MMIO バスを中心に、VDP による画面合成、VPU による 3D 描画、SGC による 2D 描画、SGU による音声、DMA・割り込み・周辺機器などを実装しています。仕様と実装の詳細は [ドキュメント一覧](#ドキュメント) を参照してください。

## 主な内容

- **CPT32 エミュレータ** — Rust で実装した CPU、メモリ／バス、描画・音声・周辺機器。
- **SRC32** — 32-bit RISC CPU。命令セット仕様は [`doc/spec_CPU.md`](doc/spec_CPU.md)。
- **sol** — スタック指向言語。Python 製 VM と SRC32 アセンブリへのコンパイラを含みます。
- **開発サンプル** — [`3d_cube.sol`](3d_cube.sol)、[`mouse_cursor.sol`](mouse_cursor.sol)、[`vpu_tnl_triangle.sol`](vpu_tnl_triangle.sol) など。

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

デバッグ GUI を有効にするには `--debug-gui` を指定します。デバッグ GUI 起動時に一時停止する場合は `--start-paused` も指定してください。
デバッグ GUI の `Windows` メニューから `Bus Composition View` を選ぶと、領域サイズをバーで比較し、その下の一覧で登録デバイスの開始・終了アドレスと容量を確認できます。未登録のアドレス範囲は一覧に表示されません。

```sh
cargo run --release --bin cpt32 -- path/to/program.bin --debug-gui --start-paused
```

CPU の対話型モニターは次のように起動します。

```sh
cargo run --release --bin cpt32 -- monitor
```

## SGU Music Editor

ROM・CPU・VDP を起動せず、独立した ImGui エディタで 16 チャンネルの曲を編集します。

```sh
cargo run --bin music_editor
cargo run --bin music_editor -- path/to/song.toml
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
- 保存形式は TOML schema v1。`format_version`, `tempo_bpm`, `ticks_per_row`, `repeat`, `orders`, `patterns`, `instruments` を持ち、pattern は 64行×16 channel。ID は0始まりです。新規曲は120 BPM、6 ticks/row、sine instrument、repeat 有効。
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

### システムと CPU

| ドキュメント | 内容 |
|---|---|
| [CPT32 システム仕様](doc/spec.md) | システム全体と各コンポーネントの設計 |
| [SRC32 CPU 仕様](doc/spec_CPU.md) | CPU、レジスタ、命令セット |
| [SRC32 ABI 仕様](doc/spec_ABI.md) | アプリケーションバイナリインターフェース |

### 言語・コンパイラ

| ドキュメント | 内容 |
|---|---|
| [sol 言語仕様](doc/spec_sol.md) | 言語、実行モデル、呼び出し規約 |
| [CSC 言語仕様](doc/spec_csc.md) | SRC32／SRC16 向け C サブセット |

### ハードウェア仕様

| ドキュメント | 内容 |
|---|---|
| [BMC / DMAC](doc/spec_BMC_DMAC.md) | バス調停と DMA |
| [VDP](doc/spec_VDP.md) | 表示タイミングと描画レイヤー合成 |
| [VPU](doc/spec_VPU.md) | 3D 描画パイプライン |
| [SGC](doc/spec_SGC.md) | スプライトと 2D コマンド描画 |
| [SGU](doc/spec_SGU.md) | 音源と音声出力 |
| [PeC](doc/spec_PeC.md) | 周辺機器コントローラー |
| [IDC](doc/spec_IDC.md) | ディスクコントローラー |

### チュートリアルと形式仕様

| ドキュメント | 内容 |
|---|---|
| [VPU の使い方](doc/VPU_howtouse.md) | `sol` から VPU を操作して描画する手順 |
| [DMA・IRQ・PeC・IDC チュートリアル](doc/DMA_IRQ_PeC_IDC_howtouse.md) | DMA と割り込みを使ったディスク読み込み |
| [SQV4 コンバーターの使い方](doc/squeezvox/README.md) | WAV と SQV4 の変換コマンド |
| [SQV4 v1/v2 仕様](doc/squeezvox/squeezvox4_spec_stub.md) | SQV4 のファイル形式と予測復号規則 |

### 設計メモ

- [LLVM IR から SRC32 アセンブリへの変換基盤計画](doc/llvm_ir_to_src32_plan.md)

### 開発ツールとエディター拡張

- [SRC16 アセンブリ用 VS Code シンタックス拡張](tools/vscode-src32-asm-syntax/README.md)
- [sol VS Code 拡張 README](tools/vscode-sol-syntax/README.md)
- [sol 拡張のクイックスタート](tools/vscode-sol-syntax/vsc-extension-quickstart.md)
- [sol 拡張の変更履歴](tools/vscode-sol-syntax/CHANGELOG.md)
