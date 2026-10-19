# CPT32 Fantasy Console Specification
## (SRC32-based Retro-Modern System)

---

# 1. Overview

本システムは、SRC32 CPUをベースとしたファンタジーコンソールであり、
2Dグラフィック（タイル/スプライト）と簡易3D（固定パイプライン）を両立する。
AmigaやPlayStation、PC-98などに影響を受けつつ、モダンな設計も取り入れる。
実際の実装では、エミュレータをRustで開発し、winit/wgpu/cpalなどのモダンなライブラリを活用する。

設計思想：

- シンプルで理解しやすいアーキテクチャ
- ハードウェア風抽象（MMIOベース）
- DMA主体による高効率処理
- レトロ + 初期3D世代の融合

---

# 2. System Components

## 2.1 System Overview

- システム名: CPT32 (仮称)
- アーキテクチャ: 32-bit レトロ・モダンハイブリッドコンソール
- 設計思想:
  - シンプルなMMIOベース構成
  - 専用ハードウェアによるオフロード
  - DMA中心の高効率データ転送

---

## 2.2 CPU

- 名称: SRC32 (Scalable RISC CPU 32-bit)
- 種別: 32-bit RISC CPU
- 内部バス: 32-bit アドレス/データバス
- クロック: 50 MHz (仮)
- 役割:
  - プログラム実行
  - 各ハードウェアの制御
  - DMA/VPU/VDPへのコマンド発行

---

## 2.3 Bus

- 内部バス幅: アドレス/データ 32-bit
- 外部バス幅: 32-bit アドレス、8/16/32-bit データアクセス
- バスアーキテクチャ: 単一バス構成
- アドレス空間: 32-bit (4GB)
- 特徴:
  - メモリマップドI/O (MMIO)
  - DMAによるバス共有
  - 単一アドレス空間

調停とDMA転送の詳細は [`spec_BMC_DMAC.md`](spec_BMC_DMAC.md) を参照。

---

## 2.4 Memory

### Main RAM
- 容量: 既定 16 MiB。`config.yaml` の `memory.ram_mib` で 1〜256 MiB に設定できる。
- 用途:
  - プログラム
  - データ
  - ワークバッファ

### VRAM
- 容量: 既定 4 MiB。`config.yaml` の `memory.vram_mib` で 4〜128 MiB に設定できる。
- 用途:
  - フレームバッファ
  - Zバッファ
  - テクスチャ
  - タイルデータ
  - スプライトデータ
  - 3Dモデルデータ

両設定を省略した場合は従来の容量を使用する。VRAM の上限はアドレス窓がPCMRAMと重ならない128 MiB。

### PCMRAM
- 容量: 1MB
- 用途:
  - PCM音源データ
  - ストリーミングバッファ

SGUの音源コア、MMIOインターフェース、レジスタマップは [`spec_SGU.md`](spec_SGU.md) を参照。

---

## 2.5 VDP (Video Display Processor)

- 機能:
  - 表示タイミングと最終画素合成
  - GP0～GP7の論理レイヤー管理（小さい番号が奥）
  - ビットマップ、PCG、SGC、VPU等の描画供給元をGP単位でsource-over合成

- 特徴:
  - 解像度: 320x240 (標準), 640x480 (拡張)
  - 文字表示: 40x30マス (標準), 80x60マス (拡張)
  - 最大8グラフィックスプレーン (GP0~GP7)。GPはフレームバッファではなく論理レイヤー
  - 属性制御 (表示/非表示、優先度、カラーモード、フリップなど)
  - タイルサイズ: 8x8 ～ 64x64
  - カラーモード:
    - 64色（標準）
    - 256色（オプション）
    - RGB555（オプション）
  - PCG機能 (キャラクタ描画、GPへの描画供給元)
    - Unicode対応 (基本多言語面まで)
    - タイルマップ制御
  - ビットマップ機能 (直接描画)
  - スクロール
  - パレット管理
  - ブレンディング(半透明)

詳細は [`spec_VDP.md`](spec_VDP.md) を参照。

---

## 2.6 SGC (Screen Graphics Controller)

- 機能:
  - VRAMの属性テーブルとパターンから2Dスプライトを描画
  - MMIO FIFOコマンドによる線分、矩形、三角形などの2D描画
  - スプライトのpriority/index順制御、flip、整数拡大縮小
  - 選択したGPへの画素出力

詳細は [`spec_SGC.md`](spec_SGC.md) を参照。回転とBitBltは初版SGC仕様に含めない。

---

## 2.7 VPU (Vector Processing Unit)

- 機能:
  - 3Dグラフィックス処理
  - 固定パイプライン方式

- サポート:
  - 三角形描画
  - 座標変換（行列）
  - ラスタライズ
  - Zバッファ

---

## 2.8 DMAC (Direct Memory Access Controller)

- 機能:
  - メモリ間高速転送
  - CPU負荷軽減

- 特徴:
  - 6チャネルDMA
  - 非同期転送
  - BMC経由のバス共有

BMCとDMACの詳細は [`spec_BMC_DMAC.md`](spec_BMC_DMAC.md) を参照。

---

## 2.9 PeC (Peripheral Controller)

- 機能:
  - 入力デバイス管理
  - イベントバッファリング

- サポート:
  - キーボード
  - マウス
  - ゲームパッド（最大4）
  - FDD / HDD インターフェース (オプション、エミュレータでは仮想ディスク)
  - その他 (MIDIなど将来拡張可能)

- 特徴:
  - FIFOバッファ
  - DMA転送対応

---

## 2.10 IRQC (Interrupt Request Controller)

- 16個のIRQ sourceの立ち上がりedgeをpendingとして保持
- 16-bit PENDING (W1C) と ENABLE mask
- 最小番号sourceを選ぶ固定優先順位 (IRQ0が最優先)
- 選択したsource番号とvalid信号をCPUへ出力
- IRQ vector、CPU割り込み許可、`IRET`はCPU側が管理。IRQCにvector/EOIレジスタはありません。

レジスタ配置とACK順序は[IRQC仕様](spec_IRQC.md)を参照。

---

## 2.11 BIOS

- 役割:
  - システム初期化
  - ハードウェア設定
  - 基本I/Oサービス提供

- 提供機能:
  - 入出力API
  - メモリ管理補助
  - デバイス初期化

- 起動方法:
  - `config.yaml` の `bios.image` に ROM ファイルを指定する（相対 path は config 基準、最大 64 KiB）。
  - raw バイナリは ROM 先頭 (`0xFF000000`) に配置され、そこから起動する。WAX はロード範囲が ROM ウィンドウ内のものだけ受け付け、entry から起動する。
  - ROM ウィンドウへの guest 書き込みは無視される。指定がなければ RAM 先頭 (`0x0`) から起動する。
---

# 3. Memory Map

## 3.1 Address Space Map

| Address Range | Size | Description | Attributes |
|--------------|------|------------|-----------|
| 0x00000000 - (0x00000000 + configured size - 1) | 1–256 MiB | Main RAM (default 16 MiB) | RWC |
| 0x10000000 - 0x17FFFFFF | up to 128 MiB configured VRAM | VRAM (default 4 MiB) | RW |
| 0x18000000 - 0x180FFFFF | 1MB | PCMRAM | RW |

### MMIO

| Address Range | Size | Device | Attributes |
|--------------|------|--------|-----------|
| 0x80000000 - 0x8000FFFF | 64KB | VDP | RWS |
| 0x80010000 - 0x8001FFFF | 64KB | SGC | RWS |
| 0x80020000 - 0x8002FFFF | 64KB | SGU x2 (SGU 2 registers at 0x80021000; shared PCMRAM) | RWS |
| 0x80030000 - 0x8003FFFF | 64KB | VPU | RWS |
| 0x80040000 - 0x8004FFFF | 64KB | PeC (UART / RNG / input / quad timer / RTC / IDC) | RWS |
| 0x80050000 - 0x8005FFFF | 64KB | DMAC | RWS |
| 0x80060000 - 0x8006FFFF | 64KB | BMC | RWS |
| 0xA0000000 - 0xAFFFFFFF | 256MB | WASM expansion card aperture (8 fixed slots; empty slots unmapped) | RWS |

WASM拡張カードはslot `n`（`0..7`）ごとに`0xA0000000 + n * 0x02000000`をbaseとする32 MiB窓を占有します。WASM module ABI、package manifest、IRQ/DMA接続は[拡張カード仕様](spec_WASM_expansion_card.md)を参照してください。

PeCのMMIOはUART、RNG、入力、クアッドタイマ、RTC、IDCの各領域を含む。タイマ／RTC詳細は[`spec_PeC.md`](spec_PeC.md)、IDC詳細は[`spec_IDC.md`](spec_IDC.md)を参照。

| PeC subdevice | Address Range | Specification |
|---|---|---|
| UART | 0x80040000 - 0x8004000F | [`spec_PeC.md`](spec_PeC.md) |
| RNG | 0x80040010 - 0x8004002F | [`spec_PeC.md`](spec_PeC.md) |
| Input | 0x80040030 - 0x8004017F | [`spec_PeC.md`](spec_PeC.md) |
| Timer global / RTC | 0x80040180 - 0x80040223 | [`spec_PeC.md`](spec_PeC.md) |
| Quad timer channels | 0x80040400 - 0x800404FF | [`spec_PeC.md`](spec_PeC.md) |
| IDC | 0x80041000 - 0x80041FFF | [`spec_IDC.md`](spec_IDC.md) |

### System

| Address Range | Size | Description | Attributes |
|--------------|------|------------|-----------|
| 0xFF000000 - 0xFF00FFFF | 64KB | BIOS ROM | R |
| 0xFFFF0000 - 0xFFFFFFFF | 64KB | CPU Reserved | RW |

---

# 4. Memory Attributes

| Attribute | Meaning |
|----------|--------|
| R | Readable |
| W | Writable |
| C | Cacheable |
| S | Side-effect |

## 4.1 Side-effect Behavior

### Read
- 状態取得
- フラグクリアなどの副作用あり

### Write
- ハードウェア動作トリガ
- DMA開始、描画開始など

---

# 5. Memory Access Rules

## 5.1 Alignment

| Access Size | Requirement |
|------------|------------|
| 8-bit  | 制限なし |
| 16-bit | 2バイト境界 |
| 32-bit | 4バイト境界 |

違反時：

- Bus Error 例外を発生

---

## 5.2 Undefined Access

未定義領域アクセス：

- Read  → Bus Error
- Write → Bus Error

---

## 5.3 Cache Policy

| Region | Cache |
|-------|------|
| RAM | Cacheable |
| VRAM | Non-cacheable |
| PCMRAM | Non-cacheable |
| MMIO | Non-cacheable |
| ROM | Read-only |

---

# 6. CPU Reserved Area

## 6.1 Vector Table

| Address | Purpose |
|--------|--------|
| 0xFFFF0000 | Reset Vector |
| 0xFFFF0004 | Illegal Instruction |
| 0xFFFF0008 | Bus Error |
| 0xFFFF0100 | Interrupt Vector (IRQC take care of IRQs) |
| 0xFFFF0104 | Syscall |

CPU予約領域のMMIO実装では、ベクターテーブルを `0xFFFF0000` から配置し、
CPUレジスタを `0xFFFF0200` から配置する。CPUレジスタは次の通り（32-bit、ビッグエンディアン）：

| Address | Register |
|--------|----------|
| 0xFFFF0200 - 0xFFFF027C | R0-R31 |
| 0xFFFF0280 | PC |
| 0xFFFF0284 | EPC |
| 0xFFFF0288 | CAUSE |
| 0xFFFF028C | STATUS (bit 0: IRQ enable) |
| 0xFFFF0290 | INSTR_MODE (0: Normal, 1: Short) |

---

# 7. Interrupt System

最低限：
- IRQ: ハードウェア割り込み
- Syscall: ソフトウェア割り込み
- 割り込み優先度制御
- IRQマスク
- 優先度レベル: 16段階
  - IRQ0 (最高) ～ IRQ15 (最低)
  - IRQ0: VBlank
  - IRQ1: DMA完了(チャネル番号はレジスタで指定)
  - IRQ2: IDC完了/エラー (PeC IDC)
  - IRQ3: PeC(I/O)イベント
  - IRQ4: タイマー
  - IRQ5: HBlank、その他デバイスイベント
  - IRQ6-15: 将来拡張用
- ネスト可能（優先度ベース）
- ACKは書き込み方式
---

# 8. BIOS

## 8.1 Role

- 初期化
- 基本I/O
- 抽象API提供

---

# 9. Implementation Notes

## 9.1 Backend (Rust)

推奨技術：

- winit (入力/ウィンドウ)
- wgpu (描画)
- cpal (音声)

## 9.2 Emulator Architecture

- CPU + Bus + Devices
- MMIOによる完全分離
- DMAベース設計

エミュレータのGUIデバッグ表示は `--debug-gui` で有効にする。ImGuiの `Windows > Performance` では、VSyncごとのCPU稼働サイクル数を設定サイクル予算で割った使用率、および実時間比を表示する。実時間比は各フレームでCPU実行に要した時間（VSync待ちと描画を除く）をエミュレーション時間で割って100倍する。100%がリアルタイム、100%未満はそれより速く、100%超は遅いことを示す。速度倍率は`100 / 実時間比`（例: 50%は2.00x、200%は0.50x）である。両指標はプログレスバーと直近120サンプルの折れ線グラフで表示する。実時間比バーは100%で満杯になり、100%超では赤色にする。折れ線グラフは0～200%に正規化し、数値表示には上限を設けない。命令実行速度（KIPS）も表示する。
`Host frame work by stage` とCPU内部のFetch/Decode/Execute/Memory/Commit/Otherは `--profile-cpu` 指定時だけ計測する。このオプションは `--debug-gui` を含み、Performanceウィンドウを起動時に開く。通常の `--debug-gui` では、追加のCPU/フレーム内訳計測は行わない。フレーム内訳はVDP tick/VSync IRQ、CPU実行、Video+ImGuiのCPU側描画送信、音声合成、音声キュー投入を最新フレームと直近120フレーム平均で表示する。CPU内訳はDevice/IRQ、Fetch、Decode、Execute、Memory、Commit、Otherのホスト時間を集計し、各段階の比率は測定されたCPU段階時間合計に対する割合である。Cached InterpreterはFetch内で命令cache lookupとdecodeも行う。描画の値はCPU側送信時間であり、GPU実行時間、VSync待ち、event-loop idleを含まない。計測自体の影響でCPUは遅くなるため、絶対性能値ではなく処理段階の相対比較に使う。
デバッグ GUI 有効時は ImGui のメインメニューバーが画面上部に描画されるため、ゲーム画面はメニューバーの高さ分だけ下にずらして表示し、メニューと重ならないようにする。マウス座標からゲスト座標への変換も同じずらし量を使う。

`Windows > Debug Controls` はCPU状態、PC、命令モード、合計サイクル数、合計実行命令数を表示する。`Run CPU (Realtime)` チェックボックスと Pause / Run / Step ボタンはエミュレーション全体の実行可否（`DebugUiState::running`）を制御する。Step 系ボタンは実行を停止した状態で少量だけ進めるため、ステップ後は Run で再開する必要がある。

`Windows > Audio Mixer` はSGU 1/2、IDCディスク音、拡張カード音声の各出力をフレーム単位で計測し、正規化したRMSとピークを表示する。各音源は個別にミュートできる。計測値はミュート中も生成された音声から計算されるため、ミュート操作は音源の状態進行を止めない。
`Debug Controls` の `Reset` はCPUをアドレス `0x00000000` から再初期化し、CPUの総サイクル数・稼働サイクル数・実行命令数を0にし、IRQコントローラの保留要求・許可・入力レベルを消去して一時停止する。ゲストRAMとバスのシステム時刻は保持する。

ここは以前ドキュメントに書かれておらず、あいまいだった点を明記する:

- **一時停止中は CPU が1命令も実行されない。** `running == false` のとき `execute_cpu` は `cpu.run` を呼ばず、`tick` → `tick_devices` → `refresh_device_irqs` が走らない。したがって IRQC の source 3 がサンプリングされず、`halt` で停止中の CPU はキーボード入力（PeC）で復帰しない。pec-input スレッドは独立して入力を FIFO へ積むため、停止中に入力を打つと FIFO が増えていき 256 件でオーバーフローする。これは障害ではなく「停止中は入力処理されない」ことの帰結だが、原因が分かりにくかった。
- **`CPU running:` 表示はあいまい。** これは `cpu.is_running()`（CPU の halt 状態）を示し、REPL 待機中の halt と一時停止の双方で `false` になる。区別するため `Emulation: RUNNING (realtime)` / `Emulation: PAUSED - CPU not ticked; keyboard input is not processed` 行を追加した。キーボードが無反応ならまず `PAUSED` を確認し、チェックを ON（または Run）すれば復帰する。一時停止中に積まれた入力は再開後に処理される。
- **フォーカス時のキー状態スナップショットは入力ではない。** winit はウィンドウがフォーカスを得る/失うたびに、その時点で OS が「押下中」と報告するキーの `KeyboardInput` を `is_synthetic == true` として合成する（`WM_SETFOCUS` / `WM_KILLFOCUS`）。IME 有効時は OS が IME 自身の仮想キー（`VK_DBE_ALPHANUMERIC` / `VK_DBE_SBCSCHAR` など）を押下中として報告し、Windows がそれを無関係なスキャンコード（Backquote / Caps Lock）に写像する。この合成イベントをゲスト入力へ流すと、**ウィンドウにフォーカスを合わせるだけで幽霊キー（HID usage 0x35 = `` ` ``、usage 0x39 = Caps Lock）が PeC FIFO に入り、ゲストが ` を表示・エコーしていた**。`guest_key_input` は `is_synthetic` のイベントを変換しない（`src/main.rs`）。フォーカス中に実キーを押しっぱなしにして戻った場合、そのキーの押下は実イベントが来るまでゲストに渡らない（離した時のイベントは届くが、キー押下を見ていないゲストでは無害）。ユーザーが半角/全角キーを実際に押した場合は今も US-HID 変換により usage 0x35 が届く（別件の既知動作）。

---

# 10. Future Extensions

- カートリッジ領域
- ネットワーク
- ストレージ
- デバッグ機構

---

# End of Specification
