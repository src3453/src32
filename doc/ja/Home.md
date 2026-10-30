---
aliases:
  - CPT32 日本語ドキュメントホーム
  - CPT32 Japanese Documentation Home
tags:
  - cpt32/wiki
  - cpt32/home
lang: ja
---

# CPT32 ドキュメントホーム

仕様・ガイド・設計資料の日本語ナビゲーションです。原文のファイル配置を保ち、分野別目次（MOC）から既存ページを Wiki リンクでたどれるようにしています。

## 分野別ナビゲーション

- [[doc/Systems|システムと CPU]] — 全体構成、SRC32、ABI、CPU 拡張
- [[doc/Hardware|ハードウェア仕様]] — バス、描画、音声、周辺機器
- [[doc/Software|言語・パッケージ・ツール]] — sol、CSC、各種形式、開発ツール
- [[doc/Guides|チュートリアルと開発ガイド]] — 使い方、SDK、設計メモ

英語の Wiki ホーム: [[doc/Home|Documentation Home]]

## 日本語版がある英語原文

各日本語版の `translation_of` frontmatter は対応する原文を示します。識別子、コード、数値、形式要件を原文と照合してください。

### CPU・実行形式

- [[doc/ja/spec_CPU|SRC32 仕様]] ↔ [[doc/spec_CPU|SRC32 Specification]]
- [[doc/ja/spec_CPU_p|SRC32 Extension p 仕様]] ↔ [[doc/spec_CPU_p|Extension p — Pipelining]]
- [[doc/ja/spec_ABI|SRC32 ABI 仕様]] ↔ [[doc/spec_ABI|SRC32 ABI Specification]]
- [[doc/ja/spec_WAX|WAX コンテナー仕様]] ↔ [[doc/spec_WAX|WAX container spec]]
- [[doc/ja/spec_WAX_v3|WAX/WAL v3 仕様]] ↔ [[doc/spec_WAX_v3|WAX/WAL v3 specification]]
- [[doc/ja/spec_COW|COW sidecar 仕様]] ↔ [[doc/spec_COW|COW sidecar format]]
- [[doc/ja/spec_CPT32PKG|CPT32PKG package 仕様]] ↔ [[doc/spec_CPT32PKG|CPT32PKG package format]]

### SQV4

- [[doc/ja/squeezvox/README|SQV4 converter ガイド]] ↔ [[doc/squeezvox/README|SQV4 file converter]]
- [[doc/ja/squeezvox/squeezvox4_spec_stub|SQV4 v3 形式仕様]] ↔ [[doc/squeezvox/squeezvox4_spec_stub|SQV4 specification]]

### 開発ツールとサンプル

- [WASM starter README（日本語）](../../templates/wasm-expansion-card/README.ja.md) ↔ [English](../../templates/wasm-expansion-card/README.md)
- [WASM timer example（日本語）](../../examples/wasm-expansion-timer/README.ja.md) ↔ [English](../../examples/wasm-expansion-timer/README.md)
- [sol VS Code README（日本語）](../../tools/vscode-sol-syntax/README.ja.md) ↔ [English](../../tools/vscode-sol-syntax/README.md)
- [sol 拡張クイックスタート（日本語）](../../tools/vscode-sol-syntax/vsc-extension-quickstart.ja.md) ↔ [English](../../tools/vscode-sol-syntax/vsc-extension-quickstart.md)

## 使い方

- リポジトリのルートを Obsidian vault として開くと、`[[wikilink]]` と分野別リンクが解決されます。
- コードや README から参照されている既存 `doc/` ページは移動していません。分野別ノートを入口として追加しています。
- 仕様を変更するときは、関連する実装・テスト・英語原文／日本語版・分野別目次も合わせて更新してください。
