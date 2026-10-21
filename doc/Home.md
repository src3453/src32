---
aliases:
  - CPT32 Documentation Home
  - CPT32 ドキュメントホーム
tags:
  - cpt32/wiki
  - cpt32/home
---

# CPT32 ドキュメントホーム

仕様・ガイド・設計資料の入口。各ページの実装根拠はリンク先の仕様と現行コードを確認してください。

日本語トップページ: [[doc/ja/Home|日本語ホーム]]

## 分野別ナビゲーション

- [[doc/Systems|システムと CPU]] — 全体構成、SRC32、ABI、CPU 拡張
- [[doc/Hardware|ハードウェア仕様]] — バス、描画、音声、周辺機器
- [[doc/Software|言語・パッケージ・ツール]] — sol、CSC、ファイル形式、開発ツール
- [[doc/Guides|チュートリアルと開発ガイド]] — 実装・利用手順、サンプル

## 言語

英語で書かれた仕様・ガイドには `doc/ja/` 内に日本語版を用意しています。各日本語版の frontmatter に `translation_of` を記載し、原文との対応を明示しています。

- [[doc/ja/spec_CPU|SRC32 仕様（日本語）]] ↔ [[doc/spec_CPU|SRC32 Specification]]
- [[doc/ja/spec_CPU_p|Extension p 仕様（日本語）]] ↔ [[doc/spec_CPU_p|Extension p — Pipelining]]
- [[doc/ja/spec_CPU_priv|Extension P 仕様（日本語）]] ↔ [[doc/spec_CPU_priv|Extension P — Privilege Levels and System Calls]]
- [[doc/ja/spec_WAX|WAX 仕様（日本語）]] ↔ [[doc/spec_WAX|WAX container spec]]
- [[doc/ja/spec_COW|COW 仕様（日本語）]] ↔ [[doc/spec_COW|COW sidecar format]]
- [[doc/ja/spec_CPT32PKG|CPT32PKG 仕様（日本語）]] ↔ [[doc/spec_CPT32PKG|CPT32PKG package format]]
- [[doc/ja/squeezvox/README|SQV4 converter（日本語）]] ↔ [[doc/squeezvox/README|SQV4 file converter]]
- [[doc/ja/squeezvox/squeezvox4_spec_stub|SQV4 形式仕様（日本語）]] ↔ [[doc/squeezvox/squeezvox4_spec_stub|SQV4 specification]]

## Vault の使い方

- このリポジトリのルートを Obsidian vault として開くと `[[wikilink]]` が解決されます。
- 既存の `doc/` 内ファイル名と配置は維持しています。コード・README からの相対リンクを壊さず、分野別ノートを目次として追加しています。
- 仕様ページを変更するときは実装とテストも確認し、関連する分野別ページと翻訳を更新してください。
