# 依存関係の更新

[Dependabot 設定](../.github/dependabot.yml) は毎週月曜 09:00（Asia/Tokyo）に更新を確認し、既定ブランチ向けの PR を作成します。設定が GitHub の既定ブランチに反映されると動作します。自動マージは設定していません。

- Cargo: 本体と、独立した WASM SDK・テンプレート・3つの実例。各ディレクトリの `Cargo.toml` / `Cargo.lock` を対象にします。
- npm: `tools/vscode-sol-syntax`。`tools/vscode-src32-asm-syntax` は依存関係を宣言していないため対象外です。
- pip: `tools/llvm-src32/requirements.txt`。
- GitHub Actions: `.github/workflows` の `uses:` 参照。

バージョン更新 PR の上限は各設定につき5件です。Cranelift の直接依存は同じ PR にまとめ、それ以外は個別にレビューします。メジャー更新も提案対象です。

## 更新 PR の確認

更新された経路に合う [開発ガイド](../AGENTS.md) の検証を実行し、CI の結果と合わせて確認します。特に次の契約を維持してください。

- Cranelift は関連 crate のバージョンを揃え、`jit` を有効にしたビルドと実行も確認します。
- `wgpu` は `imgui-wgpu` と互換性のある組み合わせが必要です。現行 `Cargo.toml` にある 30.x の互換性注意を確認し、描画の起動確認も行います。
- WASM カードは変更された実例・テンプレートを `wasm32-unknown-unknown` 向けにビルドし、カードの変更では package 化後の動作も確認します。現在の CI が全実例を検証するとは限りません。
- llvmlite は生成 IR を処理する Clang と LLVM のメジャーバージョンを揃えます。CI は llvmlite の LLVM バージョンから Clang を選びますが、ローカル環境でも確認が必要です。
- npm 更新では sol 拡張のコンパイルと VSIX 作成を確認します。
- Rust / npm の配布依存が変わったら `python scripts/generate_license_notices.py` で通知を再生成し、`python scripts/generate_license_notices.py --check` を通します。新しいライセンスや通知の不足は個別に確認します。

新しい manifest や外部依存を追加するときは Dependabot の対象も見直します。workflow 内の `pip install pytest`、`npx @vscode/vsce@3`、システムパッケージ、外部ソースなどは、この設定で manifest の依存として更新されません。

Dependabot alerts / security updates のリポジトリ設定は別管理です。このファイルだけで有効化したとは扱いません。設定項目は [GitHub の公式リファレンス](https://docs.github.com/en/code-security/reference/supply-chain-security/dependabot-options-reference) を参照してください。
