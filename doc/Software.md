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
- [[doc/spec_WAX|WAX container spec]] / [[doc/ja/spec_WAX|日本語版]] — `.wax` 実行形式 / `.wal` ライブラリ（ゲスト loader: `sol/cpt32/wax.sol`、DOS: `RUN NAME`）
- [[doc/spec_WAX_v3|WAX/WAL v3]] / [[doc/ja/spec_WAX_v3|日本語版]] — CPU 保護・依存関係・ライブラリシンボル・公開 ABI（仕様策定済み、未実装）
- [[doc/spec_Foundation|Foundation カーネル ABI]] — マイクロカーネル OS のトラップ・IPC・syscall・sol 互換 ABI（仕様確定、未実装）。[[doc/foundation_os_plan|実装案]]
- [[doc/spec_Foundation_API|Foundation API 仕様]] — ストリーム・fs セッション・コンソールのプロトコルと libfnd（主要判断確定、未実装）
- [[doc/spec_Foundation_shell|Foundation シェル仕様]] — `/bin/shell.wax` の構文・展開・リダイレクト・組み込みコマンド・スクリプト（主要判断確定、未実装）
- [[doc/spec_COW|COW sidecar format]] / [[doc/ja/spec_COW|日本語版]] — 書き込み可能 overlay
- [[doc/spec_DSFS|DSFS 仕様]] — フロッピーディスク用シンプルファイルシステム（ツール: `tools/dsfs/dsfs.py`、ゲストライブラリ: `sol/cpt32/idc.sol`, `sol/cpt32/dsfs.sol`）
- [[doc/spec_FS2|FS2 (File System 2) 仕様]] — Foundation用の階層型FS、ビットマップと連結割り当て、最大256文字の名前、UID権限、ミリ秒時刻、VFS（仕様策定、未実装）
- [[doc/squeezvox/squeezvox4_spec_stub|SQV4 v1/v2 specification]] / [[doc/ja/squeezvox/squeezvox4_spec_stub|日本語版]]

- [[doc/spec_SGUB|SGUB v1 music stream]] — 60 Hz SGU レジスタ列、Furnace ROM エクスポート
- [[doc/spec_SGUB_v2|SGUB v2 音楽ストリーム仕様案]] — pool ID参照と連続領域操作を使う Draft形式
- [[doc/spec_SGUB_v3|SGUB v3 音楽ストリーム仕様案]] — WASM拡張カードへの順序付き書き込みとS3HSカード出力

## 開発 SDK と拡張

- [[doc/spec_WASM_expansion_card|WASM Expansion Card ABI v1]]
- [[doc/WASM_expansion_card_development|WASM 拡張カード開発ガイド]]
- [拡張カード starter README](../templates/wasm-expansion-card/README.md) ([日本語](../templates/wasm-expansion-card/README.ja.md))
- [sol 拡張クイックスタート](../tools/vscode-sol-syntax/vsc-extension-quickstart.md) ([日本語](../tools/vscode-sol-syntax/vsc-extension-quickstart.ja.md))

## 設計資料

- [[doc/llvm_ir_to_src32_plan|LLVM IR から SRC32 アセンブリへの変換基盤計画]]
