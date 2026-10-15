---
aliases:
  - Software and File Formats
  - 言語・パッケージ・ツール
tags:
  - cpt32/wiki
  - cpt32/software
---

# 言語・パッケージ・ツール

[[doc/Home|ドキュメントホーム]] · [[doc/Systems|システムと CPU]] · [[doc/Hardware|ハードウェア]] · [[doc/Guides|ガイド]]

## 言語とコンパイラ

- [[doc/spec_sol|sol 言語仕様]] — VM と SRC32 コンパイラ
- [[doc/spec_csc|CSC 言語仕様]] — SRC32 / SRC16 向け C サブセット
- [sol VS Code 拡張 README](../tools/vscode-sol-syntax/README.md) ([日本語](../tools/vscode-sol-syntax/README.ja.md))
- [sol VS Code 拡張の変更履歴](../tools/vscode-sol-syntax/CHANGELOG.md) ([日本語](../tools/vscode-sol-syntax/CHANGELOG.ja.md))
- [SRC16 アセンブリ用 VS Code 拡張 README](../tools/vscode-src32-asm-syntax/README.md)

## 配布・保存形式

- [[doc/spec_CPT32PKG|CPT32PKG package format]] / [[doc/ja/spec_CPT32PKG|日本語版]] — ゲストプログラムと媒体の read-only package
- [[doc/spec_WAX|WAX container spec]] / [[doc/ja/spec_WAX|日本語版]] — `.wax` 実行形式 / `.wal` ライブラリ
- [[doc/spec_COW|COW sidecar format]] / [[doc/ja/spec_COW|日本語版]] — 書き込み可能 overlay
- [[doc/squeezvox/squeezvox4_spec_stub|SQV4 v1/v2 specification]] / [[doc/ja/squeezvox/squeezvox4_spec_stub|日本語版]]

## 開発 SDK と拡張

- [[doc/spec_WASM_expansion_card|WASM Expansion Card ABI v1]]
- [[doc/WASM_expansion_card_development|WASM 拡張カード開発ガイド]]
- [拡張カード starter README](../templates/wasm-expansion-card/README.md) ([日本語](../templates/wasm-expansion-card/README.ja.md))
- [sol 拡張クイックスタート](../tools/vscode-sol-syntax/vsc-extension-quickstart.md) ([日本語](../tools/vscode-sol-syntax/vsc-extension-quickstart.ja.md))

## 設計資料

- [[doc/llvm_ir_to_src32_plan|LLVM IR から SRC32 アセンブリへの変換基盤計画]]
