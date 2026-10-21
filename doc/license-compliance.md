# MIT ライセンスと外部コンポーネント

確認日: 2026-10-21。これはリポジトリ・依存メタデータ・同梱通知に基づく技術的な確認記録であり、全ファイルの権利関係を保証するものではありません。

## 適用範囲

CPT32 のオリジナル実装・ドキュメントにはルートの `LICENSE`（MIT）を適用します。第三者のコード・フォント・ライセンス本文は各権利者の条件を維持します。第三者の条件を MIT に置き換えるものではありません。権利未確認として以下に列挙したファイルは、確認完了までルート MIT の許諾対象に含めません。

所有者から `assets/bios.rom`、`sound.cpp`、`sound.hpp` は自作であると確認済みです。BIOS のソースは `sol/bios.sol` です。Git のコミット著者は src3453 ですが、著者一覧だけから全素材の権利を証明することはできません。

## 依存パッケージの確認結果

ルートの `cargo metadata --all-features` で取得した380外部パッケージ（推移依存・全ターゲット・build/dev依存を含む）のライセンス宣言を確認しました。MIT、Apache-2.0（LLVM例外付きも含む）、BSD、ISC、Zlib、CC0 等に加え、`option-ext` の MPL-2.0 が存在します。確認した宣言には GPL のみを必須とする Rust 依存はありません。

これらは CPT32 自作部分への MIT 採用を妨げるものとは判断していません。ただし、それぞれの通知・条件を履行して配布する必要があります。

| 対象 | 条件と対応 |
| --- | --- |
| MIT / BSD / ISC 等 | 著作権・許諾・免責条文を保持。`THIRD_PARTY_NOTICES.txt` に全文を収録。 |
| Apache-2.0 / LLVM例外 | ライセンス全文、同梱 NOTICE、例外条文を保持。変更した外部ファイルを配る場合は変更の告知も必要。 |
| `option-ext` 0.2.0 / MPL-2.0 | ファイル単位の条件を維持。未変更で使用。実行形式の受領者には対象ソースの取得方法を案内する。対象ソース: https://crates.io/api/v1/crates/option-ext/0.2.0/download 。変更版を配る場合は、その変更を含む対象ソースを提供する。CPT32 全体を MPL に変更する必要はない。 |
| OR で選択できるもの | MIT または Apache 等の許容される選択肢を利用できる。`r-efi` の LGPL 選択肢を必須とは扱わない。 |
| AND / Unicode条件 | `dpi` は Apache-2.0 AND MIT、`unicode-ident` は (MIT OR Apache-2.0) AND Unicode-3.0。片方だけで済ませない。全文を保持。 |
| Dear ImGui / stb | `imgui-sys` の同梱 Dear ImGui LICENSE と `imstb_*.h` 内の個別ライセンスも収録。Rust wrapper の表示だけでは足りない。 |
| VS Code sol 拡張 | package-lock の production 依存8パッケージ（Language Client / Protocol / Types / JSON-RPC / semver 等）を収録。拡張フォルダーにも LICENSE と通知全文を配置。開発ツールを配布する場合は別途その依存も対象になる。 |
| Berkeley SoftFloat 3e | BSD-3-Clause。既存の COPYING、ソースのヘッダー、UPSTREAM.md を保持。ゲストバイナリだけを配る場合も通知全文を同梱する。 |
| UNSCII-8 | 上流の PD / CC0 宣言を保存。CGA.F08 を削除し、UNSCII.F08 に置換。CP437 の256スロット、1文字8 byte、合計2048 byte。図柄は変わるが、文字コード・VRAM・MMIOの契約は変更しない。上流にない4文字（0x0F、0x7F、0xA9、0xF9）は `?` で代替。 |
| GNU Unifont | MIT 対象外。OFL-1.1 の選択肢を利用し、フォント部分の条件を維持。共通上流 LICENSE 全文を保存したが、下記のバージョン別権利表示確認は未完了。 |
| SVG 内の Source Sans Pro | `doc/cpt32-architecture.svg` の埋め込み WOFF の name table で Adobe 2010–2018、Reserved Font Name “Source”、OFL-1.1 を確認。`licenses/SOURCE-SANS-PRO.txt` を同梱。 |

一部の crate は公開アーカイブにライセンス本文を含めないため、`.cargo_vcs_info.json` が指定する正確な上流コミットから取得して `licenses/overrides.json` に URL と本文を保存しています。
`crc32c`、`dispatch`、`hexf-parse`、`sptr` は、その上流コミットにも独立した本文が見つかりませんでした。Cargo の宣言・作者情報と SPDX v3.26.0 の標準条文を記録しました。`r-efi` の本文は `AUTHORS` にあります。標準条文による補完は、未知の著作権表示を創作したり、上流の権利を認定したりするものではありません。

## 配布前に残る確認

- `examples/wasm-3ws88pwn4/` は Furnace の別リポジトリの音源を参照しています。独立したRust実装との説明はありますが、参照元の実装との権利関係・移植部分の再許諾権限は今回の確認では確定していません。このディレクトリを MIT と断定せず、カードを配布する前に権利者と来歴を確認してください。別リポジトリの自作ファイルを GPL プロジェクトで利用した場合と、第三者の GPL 部分を移植した場合は区別が必要です。
- 同梱 Unifont の既存記録は18.0.01を指しますが、その版の LICENSE URL は今回404でした。`https://unifoundry.com/LICENSE.txt` から共通条文を取得しました。そこには著作権者欄のプレースホルダーが残っているため、該当データのリリースに対応する実際の著作権者・Reserved Font Name・条文を確認して補完するまで、Unifont入り配布物の完全な適合確認は完了扱いにできません。
- 実行形式・生成ROM・曲・テスト素材・ユーザーが追加したカード等の全データの権利を、Cargo/npm の一覧だけで証明することはできません。特に曲・サンプルを追加して配布するときは、その素材も確認してください。86Box のドライブ音素材は従来どおり同梱しません。

## 再生成・配布

```powershell
cargo fetch
# lockfile がまだなければ cargo generate-lockfile で作成する
# tools/vscode-sol-syntax で npm ci を実行して依存の本文を用意する
python scripts/generate_license_notices.py
python scripts/generate_license_notices.py --check
python tools/build_unscii_font.py --check
cargo check --bin cpt32 --all-features --offline
```

生成器はネットワークを使わず、手元のCargo解決結果とnpm lockfileを元に動きます。`Cargo.lock` は既存運用でGit管理対象外なので、別環境で解決が変わったときや依存更新時には必ず再生成・差分確認してください。未知のライセンスや本文欠落はエラーになります。ただし、条文の収録検査は法的な適合判定を自動化するものではありません。依存の追加・条件変更は目視でも確認します。

配布物には `LICENSE`、`THIRD_PARTY_NOTICES.txt` と対象素材の通知・来歴を含めます。MPL の対象ソース取得先も通知に収録しています。GUI の `Help → About / Third-party licenses` はこの全文を実行ファイルへ埋め込むため、作業ディレクトリや外部通知ファイルがなくても読めます。メニューはデバッグUIを有効にした画面にあります。

VSIX は拡張フォルダー内の LICENSE / THIRD_PARTY_NOTICES.txt を含めて配布します。Python/LLVM/Node自体や開発用依存をバンドルする配布は今回の生成器の対象外です。例えば llvmlite の配布物を同梱する場合は、llvmlite/LLVM の通知も別途保持してください。

参照: [MIT (OSI)](https://opensource.org/license/mit)、[MPL 2.0](https://www.mozilla.org/en-US/MPL/2.0/)、[OFL 1.1](https://openfontlicense.org/open-font-license-official-text/)、[UNSCII](https://github.com/viznut/unscii)、[SPDX license texts v3.26.0](https://github.com/spdx/license-list-data/tree/v3.26.0/text)。

## 実行した検証

- `cargo check --bin cpt32 --all-features --offline`: 通過。GUIの実画面操作は未実施。
- `cargo test --test vdp_pcg_scroll --offline --all-features`: 1件通過。
- `cargo test --test unscii_font --offline --all-features`: 1件通過。NUL、ASCII A、CP437ブロック、両8×8バンクへのロード、専用8×16領域を保持することを確認。
- 通知とフォントの `--check`: 通過。未審査のライセンス文字列をCargoメタデータへ注入する検査で、生成器がエラーにすることも確認。
- `rustfmt --edition 2024 --check src/main.rs`、`git diff --check`: 通過。
