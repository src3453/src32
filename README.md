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

```sh
cargo run --release --bin cpt32 -- path/to/program.bin --debug-gui --start-paused
```

CPU の対話型モニターは次のように起動します。

```sh
cargo run --release --bin cpt32 -- monitor
```

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
