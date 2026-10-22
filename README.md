# CPT32

SRC32 CPU をベースにしたレトロ・モダン系ファンタジーコンソールと、その Rust 製エミュレータです。CPU と MMIO バスを中心に、VDP による画面合成、VPU による 3D 描画、SGC による 2D 描画、SGU による音声、DMA・割り込み・周辺機器などを実装しています。仕様と実装の詳細は [ドキュメント一覧](#ドキュメント) を参照してください。
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/src3453/src32)
## 主な内容

Windows／Linuxで起動設定を操作できるPython GUIランチャーを用意しています。`python tools/cpt32-launcher/launcher.py` で開き、ROM・CPU方式・BIOS・ディスク・WASMカード・音声・music streamを選択して起動できます。ROMに `.sol` を指定すると自動でコンパイルして実行します。設定とNamed profileは画面から保存します。[使い方と設定](tools/cpt32-launcher/README.md)を参照してください。

- **CPT32 エミュレータ** — Rust で実装した CPU、メモリ／バス、描画・音声・周辺機器。
- **SRC32** — 32-bit RISC CPU。命令セット仕様は [`doc/spec_CPU.md`](doc/spec_CPU.md)。
- **sol** — スタック指向言語。Python 製 VM と SRC32 アセンブリへのコンパイラを含みます。標準ライブラリ（`sol/solstd`、`sol/cpt32`）は名前空間を宣言しており、`pcg.prnscr` や `softfloat.sf32_add` のように `名前空間.名前` で参照します（[`doc/spec_sol.md`](doc/spec_sol.md)）。
- **CPT32 Cライブラリ** — sol版の描画・IRQ・ストレージ・音楽APIを、LLVM/SRC32向けfreestanding Cへ移植。SDKは [`sdk/cpt32`](sdk/cpt32/README.md)、サンプルは [`3D Cube`](examples/3d-cube-c/README.md)、[`Terrain Flight`](examples/vpu-terrain-flight-c/README.md)、[`DOS`](examples/dos-c/README.md)、[`TinyBASIC`](examples/tinybasic-c/README.md)。
- **C++17 + LLVM サンプル** — クラスを使う複数ファイルと C SDK をリンクし、文字画面に結果を表示。CMake／CLI の手順は [`Hello C++`](examples/hello-cpp/README.md)。
- **C 外部ライブラリデモ** — [`Berkeley SoftFloat Release 3e`](examples/softfloat-c/README.md) を組み込み、浮動小数点の四則演算・丸め・例外フラグをゲストで検証して表示。
- **Foundation（OS）** — [Foundation 仕様](doc/spec_Foundation.md)に沿った CPT32 用 OS。フェーズ1の最小カーネル（U モードのプロセス、1 ms tick と 10 tick のタイムスライスによる優先度付きラウンドロビン、保護違反でのプロセス終了）が [`foundation`](foundation/README.md) にあります。
- **ユーザーモード SYSCALL サンプル** — 拡張Pで C のカーネルが保護領域を設定し、ユーザーモードのプログラムから SYSCALL を受け付ける。[`syscall-user-c`](examples/syscall-user-c/README.md)。
- **開発サンプル** — [`3d_cube.sol`](3d_cube.sol)、[`mouse_cursor.sol`](mouse_cursor.sol)、[`vpu_tnl_triangle.sol`](vpu_tnl_triangle.sol) など。
- **CPT32 DOS / TinyBASIC** — sol版 [`dos.sol`](dos.sol)、[`tinybasic.sol`](tinybasic.sol) と、freestanding C移植 [`DOS`](examples/dos-c/README.md)・[`TinyBASIC`](examples/tinybasic-c/README.md)。

## ビルドと実行

Rust のツールチェーンが必要です。まずリリースビルドします。

```sh
cargo build --release
```

タイトルバーには `CPT32 (build 264 at 2026/10/19 00:00:00 UTC)` の形式で、ビルド元の `HEAD` から到達できるコミット数と、ビルド時のUTC日時を表示します。カートリッジ名はその後ろに表示します。Git情報のないソース配布では番号を `unknown` と表示します。浅いcloneでは取得済み履歴だけを数えるため、通算番号が必要な場合は完全な履歴でビルドしてください。変更のない `cargo build` は既存の日時を保持します。

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

CPU実行ポリシーは `--cpu-execution strict|best-effort|cycle-independent` で選びます。既定は `best-effort` です。`Strict` は `Cycles / frame` を正確に実行し、処理が重いとGUIも遅れます。`Best-effort` はサイクル上限または時間上限の早い方で停止し、未実行分を次フレームへ繰り越しません。`Cycle-independent` はサイクル上限を無視し、時間スライス内で実行します。時間上限は `--cpu-time-slice-ms` で1〜16 ms（既定8 ms）に設定でき、デバッグGUIの `Debug Controls` でも切り替えられます。時間上限は256サイクル単位で確認するソフト上限であり、1バッチ分だけ超えることがあります。
デバッグ GUI の `Windows` メニューから `Bus Composition View` を選ぶと、領域サイズをバーで比較し、その下の一覧で登録デバイスの開始・終了アドレスと容量を確認できます。未登録のアドレス範囲は一覧に表示されません。
`MMU / TLB (Extension V)` ウィンドウでは、VCTL の状態、PTBR、VFAULT、TLB 16 エントリの仮想ページ⇔物理ページ対応（R/W/X/U と次の置換対象）を確認できます。仮想アドレスを入力すると、ページ表を引いて物理アドレスを表示します（TLB ヒットか table walk かも表示。TLB は更新しません）。
`Performance` ウィンドウでは、Strict／Best-effort の `Cycles / frame` に対するCPU使用率と実時間比を確認できます。Cycle-independent では固定サイクル予算がないため、サイクル使用率は N/A です。実時間比は各フレームのCPU実行時間（VSync待ち・描画時間を除く）をエミュレーション時間で割って100倍します。100%がリアルタイム、100%未満はより速く、100%超はより遅いことを示します。速度倍率は `100 / 実時間比` です（例: 50% → 2.00x、200% → 0.50x）。両指標にプログレスバーと直近120サンプルの折れ線グラフを表示します。実時間比バーは100%で満杯になり、100%超では赤色です。実時間比グラフは0～200%スケール、数値表示に上限はありません。命令実行速度（KIPS）とIPCも表示します。
CPU内部とフレーム処理の時間内訳を取るには `--profile-cpu` を指定します。このオプションはデバッグGUIを有効にして `Performance` を開き、直近フレームと直近120フレーム平均を表示します。CPU内訳は Device/IRQ、Fetch、Decode、Execute、Memory、Commit、Other に分かれます。Cached Interpreter は Fetch にcache lookupとdecodeを含みます。フレーム内訳はVDP/VSync IRQ、CPU、CPU側の描画送信、音声合成、音声キュー投入です。GPU実行時間とVSync待ちは計測しません。`--debug-gui` だけではこれらの追加計測は動かず、計測器の時間計測は `--profile-cpu` 指定時のみ有効です。プロファイラー有効時はCPU実行が遅くなるため、絶対時間ではなく処理段階の比較に使ってください。
`Debug Controls` はCPUの状態、PC、モード、合計サイクル数、合計実行命令数に加えて、Extension P/V のシステムレジスタ（STATUS と IE/PIE/U/PU/PSM、保護モード、EPC、CAUSE とその意味、保留中IRQ、USP/UFP/UGP、VCTL/PTBR/VFAULT、リージョン RGN0〜7）を読み取り専用で表示します。`Reset` はCPUのサイクル数・稼働サイクル数・実行命令数をゼロにして、起動時と同じエントリアドレスから再初期化し、一時停止します。BIOS設定時はBIOSのエントリ（raw ROMは `0xFF000000`、WAXは宣言されたエントリ）、BIOS未設定時はプログラムのエントリ（rawバイナリは `0x00000000`）に戻ります。カートリッジを読み替えた場合は、新しい起動エントリを使います。IRQコントローラの保留要求・許可・入力レベルも消去します。ゲストRAMとバスのシステム時刻は保持します。

```sh
cargo run --release --bin cpt32 -- path/to/program.bin --debug-gui --start-paused
```

GUIの処理負荷を抑える実行例:

```sh
cargo run --release --bin cpt32 -- path/to/program.bin --cpu-execution best-effort --cpu-time-slice-ms 8
cargo run --release --bin cpt32 -- path/to/program.bin --cpu-execution cycle-independent
```
`--profile-cpu` は `--debug-gui` の代わりに指定でき、Performance ウィンドウを開いてCPU内訳を計測します。
```sh
cargo run --release --bin cpt32 -- path/to/program.bin --profile-cpu
```

エミュレーターは既定でパイプライン CPU（Extension p）で起動します。参照用のスカラー CPU は `--cpu scalar`、Cached Interpreter は `--cpu cached` で選択します。`--cpu pipeline` でパイプラインを明示できます。`config.yaml` の `cpu.microarchitecture` でも指定でき、CLI が優先されます。

実験的な高速モードは `cargo run --release --bin cpt32 -- dos.wax --cpu cached-relaxed` で試せます。YAML では `cpu.microarchitecture: cached-relaxed` を指定します。Cached の命令実行を使い、待機中のデバイス更新・入力IRQ確認を最大256サイクル単位にまとめます。タイミング依存デバイス（DMAC・IDC・IRQC・PeCなど）へのアクセス、タイマーIRQ期限、HALT／IRET、`run` の終了で同期し（RAM・VRAM・VDP・SGUなどタイミング非依存デバイスへのアクセスでは同期しない）、DMA／IDC転送中や拡張カードがある場合は毎サイクル処理に戻します。JITも同じ待機デバイス一括処理を使用します。入力IRQの観測時刻と一部のバス調停状態は通常の Cached と異なるため、厳密なタイミング検証には通常モードを使ってください。VSync／IRQ0は引き続きホストのフレームループから発生します。`--profile-cpu` とデバッグの単命令実行では通常の毎サイクル処理に戻るため、速度比較は profiler 無効で行います。既定は引き続き Pipeline です。

合成比較は `cargo run --release --bin cpu_relaxed_bench -- 800000 9` で実行できます。待機中のBMC／DMAC／PeC／timer／IDCを接続し、命令キャッシュを温めてから測定します。汎用ゲストや起動途中のheadless比較には `src32_testbench PROGRAM --allow-running --frames N --skip-render-checks --cpu cached-relaxed` が使えます。`--skip-render-checks` は描画assertionを省き、実行位置・R1・停止状態を報告します。停止条件と明示的なメモリ検査は維持します。[測定結果と制約](doc/emulator-performance-plan.md#実験的cached-relaxedモード)も参照してください。

実験的な Cranelift JIT は `cargo run --release --features jit --bin cpt32 -- program.bin --cpu jit` で利用できます。同じ build なら YAML の `cpu.microarchitecture: jit` でも指定できます。Normal mode の整数演算・分岐・乗除算・RAMへのload/storeを既定16命令（`cpu.jit_block_size` で2〜256）のブロックとしてコンパイルし、Short mode・割り込み復帰・MMIOアクセスなどは Cached 経路で実行します。hotなブロックは待機区間でnativeのまま連続実行し、待機中のデバイス更新は最大256サイクル単位にまとめます。MMIO、タイマー期限、DMA／IDC動作中、拡張カード、HALT／IRET、実行予算終端で同期します。高速化は保証しません。`--profile-cpu` とデバッガーの単一ステップでは毎サイクル更新の Cached 経路を使います。JIT feature がない build は `--cpu jit` をエラーにします。

JITの頻出ブロックは128回目のdispatchで一括実行用コードも生成します。コード世代とdecode cacheの一致を確認し、デバイス更新・IRQ期限のない区間で最大16命令のRust往復とregister同期をまとめます。cold fetch、変更されたコード、有効なIRQ pending、DMA／IDC動作中、拡張カードありでは保守的な経路を使います。1命令だけのブロックは解釈実行します。

比較は `cargo run --release --features jit --bin cpu_jit_bench -- 400000 7` で実行できます。compile を含む cold 時間と、同じ CPU を継続実行する warm 時間を別々に表示し、Cached・cached-relaxed・JITのレジスタ・PC・実行命令数・メモリ結果を照合します。末尾に `--platform` を加えると、待機中のBMC／DMAC／PeC／timer／IDCを接続して測定します。[JIT の設計と制約](doc/cpu-jit-design.md)も参照してください。

Cached Interpreter は RAM 上の命令を直接マップ型のデコードキャッシュに保持します。RAM の該当 4 KiB ページが書き換わるとキャッシュを無効化します。キャッシュヒット時は命令フェッチのバスアクセスを省略します。通常命令は1サイクル、MUL/MULHは3サイクル、DIV/DIVU/MODは16サイクルです。既定 CPU は変更していません。

3種類の CPU を同じループワークロードで比較するには、リリースビルドでベンチマークを実行します。位置引数は順にループ反復数と測定サンプル数です（既定値: `10000`, `5`）。実行ごとにレジスター・PC・データメモリー・完了命令数を照合し、差異があれば失敗します。

```sh
cargo run --release --bin cpu_bench -- 10000 5
cargo run --release --bin cpu_bench -- 10000 5 --platform
```

ワークロードは繰り返し実行される依存 ALU 演算、MUL とその直後の利用、ストア／ロード、load-use 依存、ループ分岐を含みます。出力は各モードの中央値、命令キャッシュ hit/miss、Cached の Scalar／Pipeline に対する speedup を示します。実効MHzは稼働サイクル数÷CPU実行秒÷100万で、48 MHz目標への達成率も表示します。既定はRAMのみ、`--platform`では待機中のBMC／DMAC／SGU／VDP／PeC入力・timer／RNG／未接続IDCも接続します。active DMA・disk・拡張カード、描画・音声処理は含みません。wall-clock 性能はホスト環境依存です。CPU サイクル数と CPI はゲスト側のカウンターです。

Headless ゲスト実行で CPU stage の内訳を採るには `src32_testbench` に `--profile-cpu` を付けます。計測器の時間計測は実行を遅くするため、内訳採取と wall-clock 比較は別々に実行してください。ホスト固有の計測結果と改善手順は [エミュレータ性能調査と改善計画](doc/emulator-performance-plan.md) を参照してください。

CPU以外の主要コンポーネントを切り分けて測るには、release ビルドで次を実行します。

```sh
cargo run --release --bin emulator_component_bench
```

Bus tick（デバイス構成別）、VDP/PCG合成、SGCのfill/sprite、VPU三角形、SGUの無音/ノイズ音声をそれぞれ測ります。GPU upload、disk/DMA動作、IDC disk audio は含まれないため、利用環境の frame profile と合わせて判断してください。

ログは `tracing` から標準エラーへ出力され、端末ではレベルごとに色分けされます。既定のレベルは `info` です。`config.yaml` の `logging.level` で `trace` / `debug` / `info` / `warn` / `error` / `off` を指定できます。`RUST_LOG` が設定されている場合はそちらを優先します。

```sh
RUST_LOG=debug cargo run --release --bin cpt32 -- path/to/program.bin
```

PCG の raw font file は引数ではなく、起動時の `config.yaml` で指定します。`font.bin` は PCG font bank 0/1 用の2 KiBまたは4 KiBファイルです。相対 path は `config.yaml` のある directory を基準にします。設定を省略した場合は内蔵フォントを使用します。

```yaml
pcg:
  font: "assets/UNSCII.F08"
```

旧来の `<program.bin> [font.bin]` 形式は使えません。

同梱の8×8フォントは PD / CC0 の UNSCII です。旧 `assets/CGA.F08` を指定していた設定は `assets/UNSCII.F08` に変更してください。文字配置は CP437 を維持しますが、図柄は変わります。

CPU の対話型モニターは次のように起動します。

```sh
cargo run --release --bin cpt32 -- monitor [program.bin] [--cpu scalar|cached|pipeline]
```

## CPT32 DOS（DSFS シェル）

`dos.sol` は DSFS ボリュームを操作する小さなシェルです。文字は PCG コンソールに表示し、キーボードは PeC の入力 FIFO を IRQC source 3 で待って読みます。ファイルはドライブ 0／1 の DSFS ボリューム（`doc/spec_DSFS.md`）として扱われ、`FORMAT` はゲスト自身が superblock・bitmap・directory を書きます。
同じシェルのfreestanding C移植は [`examples/dos-c/README.md`](examples/dos-c/README.md) を参照してください。LLVM/SRC32ツールチェーンと `sdk/cpt32` を使い、`csc` は使いません。

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

## TinyBASIC

[`tinybasic.sol`](tinybasic.sol) は PCG コンソールと PeC キーボードを使う sol 版インタプリタです。freestanding C 移植の機能・ビルド・実行方法は [`examples/tinybasic-c/README.md`](examples/tinybasic-c/README.md) を参照してください。C 版は LLVM/SRC32 ツールチェーンと `sdk/cpt32` を使い、`csc` は使いません。


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

### SGUB v1/v2 とゲスト再生

SGUB header は20 byte: `SGUB`, version=1, voices=16, flags（bit0 repeat）, reserved=0、big-endian u32 の frame count / initial record count / total size。
initial record の後に各 frame の big-endian u16 record count と record を格納します。
record は target u8 / address u24 big-endian / value u8 の5 byte。target 0/1 は各 SGU の offset `0..8FF`、target 2 は共有 PCMRAM の offset `0..FFFFF`。
不正 header、範囲外 address、切り詰め、余剰 byte は再生前に拒否します。VSYNC は60 Hz、tracker tick は accumulator に BPM を加え150ごとに発生します。

Music Editorはv1を出力し、FurnaceのROM exporterはpool参照方式のv2を出力します。共通driverは両方に対応します。v2の詳細は [SGUB v2仕様](doc/spec_SGUB_v2.md) を参照してください。古いdriver binaryやSGUBと同じ場所にあるdriver sourceのコピーを使用している場合は、最新の `sol/cpt32/sgu_music_driver.sol` に更新し再コンパイルしてください。SGUBの再exportは不要です。

```sh
python3 tools/solc/solc.py compile sol/cpt32/sgu_music_driver.sol -O speed --out target/sgu_music_driver.asm
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
- [依存関係の更新（Dependabot）](doc/dependency-updates.md)

英語原文があるページには `doc/ja/` 以下に対応する日本語版があります。

## ライセンス・謝辞

CPT32 のオリジナル部分は [MIT License](LICENSE) です。外部コード・フォントは各ライセンスを維持します。[謝辞](ACKNOWLEDGEMENTS.md)、[ライセンス全文](THIRD_PARTY_NOTICES.txt)、[確認結果と未確認範囲](doc/license-compliance.md) を参照してください。GUI の `Help → About / Third-party licenses` からも通知を読めます。
