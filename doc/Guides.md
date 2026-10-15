---
aliases:
  - Tutorials and Development Guides
  - チュートリアルと開発ガイド
tags:
  - cpt32/wiki
  - cpt32/guides
---

# チュートリアルと開発ガイド

[[doc/Home|ドキュメントホーム]] · [[doc/Systems|システムと CPU]] · [[doc/Hardware|ハードウェア]] · [[doc/Software|ソフトウェア]]

## 利用チュートリアル

- [[doc/VPU_howtouse|VPU の使い方]] — `sol` から FIFO に描画コマンドを送る
- [[doc/DMA_IRQ_PeC_IDC_howtouse|DMA・IRQ・PeC・IDC]] — 割り込みと DMA によるディスクアクセス
- [[doc/squeezvox/README|SQV4 converter]] / [[doc/ja/squeezvox/README|日本語版]] — PCM WAV の変換

## 開発ガイド

- [[doc/WASM_expansion_card_development|WASM 拡張カード開発ガイド]] — Rust `no_std` SDK、package、host API
- [拡張カード starter](../templates/wasm-expansion-card/README.md) ([日本語](../templates/wasm-expansion-card/README.ja.md))
- [仮想 cycle timer の実装例](../examples/wasm-expansion-timer/README.md) ([日本語](../examples/wasm-expansion-timer/README.ja.md))
- [sol VS Code 拡張 README](../tools/vscode-sol-syntax/README.md) ([日本語](../tools/vscode-sol-syntax/README.ja.md))
- [拡張のクイックスタート](../tools/vscode-sol-syntax/vsc-extension-quickstart.md) ([日本語](../tools/vscode-sol-syntax/vsc-extension-quickstart.ja.md))

## 設計資料

- [[doc/llvm_ir_to_src32_plan|LLVM IR → SRC32 変換基盤の計画]]
- [[doc/Systems|CPU・システム仕様]] · [[doc/Hardware|デバイス仕様]] · [[doc/Software|言語・ファイル形式]]
