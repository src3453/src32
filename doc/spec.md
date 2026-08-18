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

---

## 2.4 Memory

### Main RAM
- 容量: 16MB
- 用途:
  - プログラム
  - データ
  - ワークバッファ

### VRAM
- 容量: 4MB
- 用途:
  - フレームバッファ
  - Zバッファ
  - テクスチャ
  - タイルデータ
  - スプライトデータ
  - 3Dモデルデータ

### PCMRAM
- 容量: 1MB
- 用途:
  - PCM音源データ
  - ストリーミングバッファ

---

## 2.5 VDP (Video Display Processor)

- 機能:
  - 2D描画
  - タイル（PCG）モード
  - ビットマップモード

- 特徴:
  - 解像度: 320x240 (標準), 640x480 (拡張)
  - 文字表示: 40x30マス (標準), 80x60マス (拡張)
  - 最大8グラフィックスプレーン (GP0~GP7) `!現在の実装ではGP0のみ`
  - 属性制御 (表示/非表示、優先度、カラーモード、フリップなど)
  - タイルサイズ: 8x8 ～ 64x64
  - カラーモード:
    - 64色（標準）
    - 256色（オプション）
    - RGB555（オプション）
  - PCG機能 (キャラクタ描画)
    - Unicode対応 (基本多言語面まで)
    - タイルマップ制御
  - ビットマップ機能 (直接描画)
  - スクロール
  - パレット管理
  - ブレンディング(半透明)

---

## 2.6 SC (Sprite Controller)

- 機能:
  - スプライト描画管理
  - Z順制御
  - 属性管理
  - 拡大/縮小/回転
  - BitBlt(Blitter)機能

- BitBlt機能:
  - メモリコピー
  - 塗りつぶし
  - 透明転送
  - マスク処理

---

## 2.7 SGU (Sound Generator Unit)

- 型式: 3FS84PN4
- 音源コアは 3HS88PWN4 のサブセット + 追加機能
- 機能:
  - FM音源
  - PCM / ノイズ生成
  - 16bit 48KHz リニアPCM 2Chステレオ出力
  - 音量制御: 256段階
  - パンポット制御: 16段階 (左、右ともに0-15)

### FM音源
- 方式: 4オペレータ
- ハードウェアエンベロープ: アタック、ディケイ、サステイン、リリース (ADSR) 制御可能
- 波形: 16種類 (正弦波、三角波、矩形波、ノコギリ波、ランダムノイズなど)
- チャンネル数: 8ch

### PCM / Noise
- チャンネル数: 4ch
- データソース: PCMRAM
- データ形式: 8bit/16bitリニアPCM / SqueezVOX 4 1ch
- ループ制御: 可能 (ループ開始アドレス、ループ終了アドレス指定)
- サンプリングレート: 可変
- ノイズ: LSFR生成

### SqueezVOX 4 (フォーマット名 SQV4)
- ADPCM方式の音声コーデック
- SQV4形式のデータをPCMRAMに配置し、SGUで再生可能
- バリアント:
  - SQV4H: 4bit ADPCM
  - SQV4L: 3bit ADPCM
  - SQV4L+: 3bit ADPCM + 非線形量子化
- データフォーマット:
各サンプルは64サンプルの「フラグメント」に分割され、各フラグメントはヘッダとデータで構成される。
  - ヘッダ: 1バイト (音量情報)
  - データ: 3/4ビットのADPCMコードが64サンプル分 (24/32バイト)
  - 出力: 16bitリニアPCMにデコードされて再生される
---

## 2.8 VPU (Vector Processing Unit)

- 機能:
  - 3Dグラフィックス処理
  - 固定パイプライン方式

- サポート:
  - 三角形描画
  - 座標変換（行列）
  - ラスタライズ
  - Zバッファ

---

## 2.9 DMAC (Direct Memory Access Controller)

- 機能:
  - メモリ間高速転送
  - CPU負荷軽減

- 特徴:
  - 6チャネルDMA
  - 非同期転送
  - バス共有制御

---

## 2.10 PeC (Peripheral Controller)

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

## 2.11 IRQC (Interrupt Request Controller)
- 機能:
  - 割り込み管理
  - 優先度制御
  - ベクタ割り当て
  - 16レベルの割り込み優先度
  - 割り込みマスク機能
  - IRQ ACK, EOI信号管理
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

---

# 3. Memory Map

## 3.1 Address Space Map

| Address Range | Size | Description | Attributes |
|--------------|------|------------|-----------|
| 0x00000000 - 0x00FFFFFF | 16MB | Main RAM | RWC |
| 0x10000000 - 0x103FFFFF | 4MB  | VRAM | RW |
| 0x18000000 - 0x180FFFFF | 1MB  | PCMRAM | RW |

### MMIO

| Address Range | Size | Device | Attributes |
|--------------|------|--------|-----------|
| 0x80000000 - 0x8000FFFF | 64KB | VDP | RWS |
| 0x80010000 - 0x8001FFFF | 64KB | SC | RWS |
| 0x80020000 - 0x8002FFFF | 64KB | SGU | RWS |
| 0x80030000 - 0x8003FFFF | 64KB | VPU | RWS |
| 0x80040000 - 0x8004FFFF | 64KB | PeC | RWS |
| 0x80050000 - 0x8005FFFF | 64KB | DMAC | RWS |

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
  - IRQ2: HBlank (デフォルトは無効化、必要に応じて有効化)
  - IRQ3: PeC(I/O)イベント
  - IRQ4: タイマー
  - IRQ5: その他デバイスイベント
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

---

# 10. Future Extensions

- カートリッジ領域
- ネットワーク
- ストレージ
- デバッグ機構

---

# End of Specification