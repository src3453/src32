---
aliases:
  - Hardware Specifications
  - ハードウェア仕様
tags:
  - cpt32/wiki
  - cpt32/hardware
---

# ハードウェア仕様

[[doc/Home|ドキュメントホーム]] · [[doc/Systems|システムと CPU]] · [[doc/Software|ソフトウェア]] · [[doc/Guides|ガイド]]

## バスと割り込み

- [[doc/spec_BMC_DMAC|BMC / DMAC]] — バス調停と DMA 転送
- [[doc/spec_IRQC|IRQC]] — IRQ source、pending、優先順位
- [[doc/spec_PeC|PeC]] — 周辺機器コントローラー
- [[doc/spec_IDC|IDC]] — 仮想ディスクコントローラー
- [[doc/DMA_IRQ_PeC_IDC_howtouse|DMA・IRQ・PeC・IDC チュートリアル]]

## 描画

- [[doc/spec_VDP|VDP]] — 表示タイミングとレイヤー合成
- [[doc/spec_VPU|VPU]] — 3D 描画パイプライン
- [[doc/spec_SGC|SGC]] — スプライトと 2D コマンド描画
- [[doc/VPU_howtouse|VPU の使い方]]

## 音声

- [[doc/spec_SGU|SGU]] — 音源コアと PCM（v2.1: 2-tap 補間と FIFO DAC、`sgu.version` で v1/v2/v2.1 切替）
- [[doc/squeezvox/squeezvox4_spec_stub|SQV4 v1/v2 ファイル形式]]
- [[doc/squeezvox/README|SQV4 コンバーターの使い方]] / [[doc/ja/squeezvox/README|日本語版]]

## 拡張カード

- [[doc/spec_WASM_expansion_card|WASM Expansion Card ABI v1]] — 日本語原文
- [[doc/WASM_expansion_card_development|WASM 拡張カード開発ガイド]]
- 対応する実装例: [タイマー拡張カード](../examples/wasm-expansion-timer/README.md) ([日本語](../examples/wasm-expansion-timer/README.ja.md))

詳細な利用手順は [[doc/Guides|チュートリアルと開発ガイド]] を参照。
