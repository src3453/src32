# CPT32 開発ガイド

このリポジトリは SRC32 ベースのファンタジーコンソールと Rust 製エミュレータ、sol 言語の VM／コンパイラ、周辺ツールを含みます。変更時はまず該当仕様と既存実装を読み、現在の契約・テスト・隣接コードの流儀に合わせてください。挙動変更では仕様・実装・テスト・利用者向けドキュメントを一緒に更新します。

ブランチをいちいち新しく切るとマージが大変なので、作業中の変更は `main` に直接コミットして構いません。作業終了後に、必要に応じて `git rebase -i` でコミットを整理してください。

リモート環境で作業する場合は、変更を作業用ブランチにコミットしてリモートへ Push し、`main` を対象とする PR を作成してください。この場合、PR 作成に必要なブランチの作成と Push は個別の指示なしで実行します。

**作業終了後は、その作業で身につけた知識や注意点を `memory/` に新規ファイルとして書いてください（`memory/YYYY-MM-DD-issue-NNN-slug.md`、書式は `memory/README.md`）。将来の開発者が同じ間違いをしないように、契約やテストの意図を明示することが目的です。`MEMORY.md` は過去分のアーカイブで、並行する PR とのコンフリクトを避けるため新規追記しません。索引ファイルも作りません。作業終了後は、明示しない限り、コミットを実行してください。リモートへのPushは、明示しない限り行わないでください。**

## GitHub Issues とマイルストーン

進捗は GitHub の Issues（https://github.com/src3453/src32/issues ）とマイルストーンで管理します。Projects は使いません。以前使っていた Jira（プロジェクト `KAN`）は廃止しました。MEMORY.md や過去の課題に残る `KAN-n` は `doc/jira_to_github_issues.md` の対応表で Issue 番号に読み替えます。実装・機能変更・不具合修正のタスクは、特に明記しない限り Issue として載せます。ただし、プロジェクトの機能に影響しない軽微な変更は Issue の登録・更新を不要とします。例として、ブランチ操作、マージ、機械的なコンフリクト解消、コミット整理・rebase、誤字修正、開発ガイドの軽微な更新が該当します。実装や不具合修正に付随する Git 操作も、別の Issue として登録しません。競合解消で仕様や挙動の判断・変更を伴う場合は、該当する実装・不具合の Issue で管理します。GitHub の MCP サーバーや `gh` に接続できる場合だけ、次のとおり更新します。

- **作業を始めるとき**: 先に Issue をタイトルで検索し、同じ作業の Issue がなければ新しく作成します。重複する Issue を作らないでください。見つけたバグも Issue として追加し、`bug` ラベルを付けます。
- **エピックは使いません**: 親子 Issue（sub-issue）や追跡用 Issue で束ねません。分野の分類はマイルストーンで行い、すべての Issue（と PR）に次のいずれか 1 つを割り当てます。「Foundation OS」（Foundation OS 本体）、「開発基盤 (ツールチェーン、CI/CD)」（CI・ツール）、「エミュレータ・UI/UX」、「SRC32 CPU」（CPU ISA や実装の変更）。マイルストーンはリポジトリ所有者が作成し、新しい分野が必要なときは所有者に依頼します。ラベルは状態と種別（`bug`、`in-progress`、`in-review`、`on-hold`）だけに使います。
- **状態**: Issue の open／closed とラベルで表します。着手したら `in-progress`、PR を作成したら `in-review` ラベルを付け、マージしたら Issue を close します（PR 本文に `Closes #n` と書けば自動で閉じます）。中断・待機するときは `on-hold` を付けます。着手前は何も付けません。
- **PR リンク**: PR を作成したら、Issue の本文またはコメントにその PR のリンクを書きます。複数の PR にまたがる場合はすべて並べます。
- **記述**: Issue のタイトルと説明は日本語で書きます。
- **接続できない環境**: Issue の更新を省略し、そのことを作業報告に書きます。

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

- sol compiler／LSP: `python3 -m pytest tools/solc/test_sol_compiler.py tools/solc/test_sol_language_server.py tools/solc/test_sol_foundation.py`（Foundation の実行テストは `src32_testbench` がないと skip）
- sol VS Code 拡張（`tools/vscode-sol-syntax/` で実行）: `npm run compile`、`npx vsce package --no-dependencies`
- PCG scroll: `cargo test --test vdp_pcg_scroll`
- Foundation WAX ローダ（ホスト）: `python3 -m unittest discover -s foundation/tests/wax`。testbench での実行は `tools/llvm-src32/test_toolchain.py` の `test_foundation_wax_loader`
- Foundation シェル（testbench、PCG 画面の比較）: `python3 -m unittest discover -s foundation/tests/shell`（`src32_testbench` と Clang が必要）
- Foundation 仮想記憶（`mm` 層。V のある CPU では仮想記憶、`FND_MM=slot` ではスロット）: `cd tools/llvm-src32 && python3 -m unittest test_toolchain.ToolchainTests.test_foundation_vm_kernel`。シェルのテストは `python3 -m unittest discover -s foundation/tests/shell` と `FND_MM=slot python3 -m unittest discover -s foundation/tests/shell` の両方。カーネルがユーザーアドレスを直接参照せず `mm_copy_*` を使う契約は MEMORY.md を参照
- Foundation CPU 統計（ホスト、カウンタ wrap と user/kernel/idle の帰属）: `python3 -m unittest discover -s foundation/tests/cpu`（ホスト C コンパイラが必要）
- Foundation ディスク起動（BIOS → stage1 → FNDBOOT、testbench）: `python3 -m unittest discover -s foundation/tests/disk`（`src32_testbench` と Clang が必要）
- FS2 ホストツール: `python3 -m unittest discover -s tools/fs2`
- WASM SDK package builder: `python3 -m unittest discover -s sdk/wasm-expansion-card -p 'test_*.py' -v`
- WASM starter／example: `cargo build --manifest-path templates/wasm-expansion-card/Cargo.toml --release --target wasm32-unknown-unknown` と同じ形式で example の manifest も build。
- sol ゲストの実行確認は、必要に応じて `tools/solc/solc.py compile` → `tools/asm/asm.py` → `src32_testbench` またはエミュレータの順に行います。

フル Rust テストや全体の `cargo fmt --check` が golden include file の欠落や未変更コードの既存差分で失敗する場合があります。focused な結果と切り分け、実際に実行していない検証を通過したと報告しないでください。
