# CPT32 開発ガイド

このリポジトリは SRC32 ベースのファンタジーコンソールと Rust 製エミュレータ、sol 言語の VM／コンパイラ、周辺ツールを含みます。変更時はまず該当仕様と既存実装を読み、現在の契約・テスト・隣接コードの流儀に合わせてください。挙動変更では仕様・実装・テスト・利用者向けドキュメントを一緒に更新します。

## 主要な場所

- `src/`: CPU、バス、VDP／VPU／SGC／PCG、周辺機器、GUI。
- `doc/`: 仕様と使い方。特に `spec_sol.md`、各 `spec_*.md`、開発ガイドを実装の根拠にします。
- `tools/solc/`: sol の Python VM、コンパイラ、LSP とテスト。
- `tools/vscode-sol-syntax/`: sol VS Code 拡張。サーバー実装を同梱しています。
- `sol/`: ゲスト向け標準ライブラリと CPT32 用コード。
- `sdk/wasm-expansion-card/`, `templates/wasm-expansion-card/`, `examples/wasm-expansion-timer/`: WASM 拡張カード SDK、スターター、実例。

## 変更時に保つべき契約

### sol と VS Code 拡張

- 言語仕様は `doc/spec_sol.md`、実際の挙動は `tools/solc/` のコンパイラ／VMを確認してから変更します。
- sol の比較は真が `0`、偽が `1` で、`if` の then 節および `while` の反復条件は `0` で実行されます。通常の真偽値規約を前提に条件式を書かないでください。
- 関数引数への代入は現在の呼び出しフレーム内の引数スロットを更新します。ローカル変数との名前解決ではローカルを先に扱い、引数の変更が呼び出し元へ漏れないことを保ちます。
- コンパイラ／VM の意味を変えたら、VS Code 拡張内の同梱コピー（server の compiler／VM／LSP）も同期します。LSP の `Content-Length` フレーミングと標準出力のクリーンさを壊さないでください。
- IEEE-754 binary32 の sol 実装は `sol/solstd/softfloat.sol` にあります。演算・比較の変更では丸め、subnormal、無限大、NaN、符号付きゼロを含む境界を確認します。

### 描画・タイミング・ゲストコード

- GP と SGC は同じ CLUT を使います。SGC の文字色が使う先頭 64 色を維持し、GP ノイズ等の専用色には残りの色を使います。
- VSync／IRQ0 はエミュレータのホスト側フレームループから発生します。ゲストのフレーム処理やテストベンチを CPU サイクル数からの独自 IRQ 生成に合わせないでください。
- PCG の scroll X/Y は 16-bit ピクセルオフセットで、MMIO は低位 byte から書きます。表示座標は画面サイズで wrap します。コンソールの行スクロールでは物理 ring row とカーソル位置を分け、再利用する行だけを消去します。
- SGC のコマンド面はフレームをまたいで保持されます。FIFO clear は描画済み画素を消さず、画面全体を消す場合は `SCREEN_FILL` を使います。

### 設定・アーカイブ・拡張カード

- runtime config、拡張カード manifest、SGU music project の形式は YAML です。既存の YAML スキーマと serializer に合わせ、同じ用途へ TOML 形式を追加しないでください。
- ZIP 展開はしない前提で安全性を保ち、パス・サイズ・参照先を検証します。ZIP ライブラリがファイル名を map 化する場合、重複エントリ検出をその map だけに任せず raw central-directory も確認します。CPT32PKG の契約は `doc/spec_CPT32PKG.md` を参照してください。
- WASM カードは `no_std` の `Card: Default` を実装し、`export_card!(Type)` を一度だけ呼び出して ABI export を生成します。package 作成には SDK の `package.py` を使い、カード実装・manifest・asset の規則は `doc/WASM_expansion_card_development.md` に従います。

## 検証

変更した経路に合う focused check を実行し、可能なら生成物や実行時の挙動も確認します。

- sol compiler／LSP: `python3 -m pytest tools/solc/test_sol_compiler.py tools/solc/test_sol_language_server.py`
- sol VS Code 拡張（`tools/vscode-sol-syntax/` で実行）: `npm run compile`、`npx vsce package --no-dependencies`
- PCG scroll: `cargo test --test vdp_pcg_scroll`
- WASM SDK package builder: `python3 -m unittest discover -s sdk/wasm-expansion-card -p 'test_*.py' -v`
- WASM starter／example: `cargo build --manifest-path templates/wasm-expansion-card/Cargo.toml --release --target wasm32-unknown-unknown` と同じ形式で example の manifest も build。
- sol ゲストの実行確認は、必要に応じて `tools/solc/solc.py compile` → `tools/asm/asm.py` → `src32_testbench` またはエミュレータの順に行います。

フル Rust テストや全体の `cargo fmt --check` が golden include file の欠落や未変更コードの既存差分で失敗する場合があります。focused な結果と切り分け、実際に実行していない検証を通過したと報告しないでください。
