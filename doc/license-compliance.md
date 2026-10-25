# MIT ライセンスと外部コンポーネント

確認日: 2026-10-21。これはリポジトリ・依存メタデータ・同梱通知に基づく技術的な確認記録であり、全ファイルの権利関係を保証するものではありません。

## 適用範囲

CPT32 のオリジナル実装・ドキュメントにはルートの `LICENSE`（MIT）を適用します。第三者のコード・フォント・ライセンス本文は各権利者の条件を維持します。第三者の条件を MIT に置き換えるものではありません。

所有者から `assets/bios.rom`、`sound.cpp`、`sound.hpp` は自作であると確認済みです。さらに3WS88PWN4も所有者自身のプロジェクトであり、MIT適用に問題ないとの確認を受けました。`examples/wasm-3ws88pwn4/` はルートMITの適用範囲に含め、Cargoの宣言と単体配布用LICENSEも追加しています。BIOS のソースは `bios/main.c`（C 版、`assets/bios.rom` の生成元）と旧 sol 版の `sol/bios.sol` です。Git のコミット著者は src3453 ですが、著者一覧だけから全素材の権利を証明することはできません。

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
| Unicode CaseFolding 16.0.0 | `tools/fs2/CaseFolding-16.0.0-CF.txt` は Unicode, Inc. のデータ（C と F の行だけを抜き出したもの）。Unicode License V3 の著作権・許諾表示をファイル先頭に保持。FS2 の名前照合（spec_FS2.md §6）に使う。 |
| Berkeley SoftFloat 3e | BSD-3-Clause。既存の COPYING、ソースのヘッダー、UPSTREAM.md を保持。ゲストバイナリだけを配る場合も通知全文を同梱する。 |
| LZMA SDK 26.05 | Igor Pavlov による public domain の C 部分を `foundation/lib/lzma/vendor/` に収録。原文 `lzma-sdk.txt`、各ソースの宣言、取得元・アーカイブ SHA-256 を保持。CPT32 のアダプターと区別し、SDK を MIT に再ライセンスしない。 |
| UNSCII-8 | 上流の PD / CC0 宣言を保存。CGA.F08 を削除し、UNSCII.F08 に置換。CP437 の256スロット、1文字8 byte、合計2048 byte。図柄は変わるが、文字コード・VRAM・MMIOの契約は変更しない。上流にない4文字（0x0F、0x7F、0xA9、0xF9）は `?` で代替。 |
| Jiskan16 | パブリックドメイン。X.Org font-jis-misc 1.0.4 の未変更BDFと COPYING を同梱。JIS X 0208 の6,877字形を共通ROMへ上書き。出典・SHA-256は `assets/JISKAN16-LICENSE.txt` を参照。 |
| GNU Unifont | MIT 対象外。OFL-1.1 の選択肢を利用。公式18.0.01アーカイブの標準BDFから著作権表示を保存し、同梱HEXとの一致を確認。リリースのOFL全文も同梱。変換したROMもOFL-1.1を維持。 |
| SVG 内の Source Sans Pro | `doc/cpt32-architecture.svg` の埋め込み WOFF の name table で Adobe 2010–2018、Reserved Font Name “Source”、OFL-1.1 を確認。`licenses/SOURCE-SANS-PRO.txt` を同梱。 |

一部の crate は公開アーカイブにライセンス本文を含めないため、`.cargo_vcs_info.json` が指定する正確な上流コミットから取得して `licenses/overrides.json` に URL と本文を保存しています。
`crc32c`、`dispatch`、`hexf-parse`、`sptr` は、その上流コミットにも独立した本文が見つかりませんでした。Cargo の宣言・作者情報と SPDX v3.26.0 の標準条文を記録しました。`r-efi` の本文は `AUTHORS` にあります。標準条文による補完は、未知の著作権表示を創作したり、上流の権利を認定したりするものではありません。

## Unifont の確認完了

公式の [Unifont ページ](https://unifoundry.com/unifont/index.html) と [LICENSE.txt](https://unifoundry.com/LICENSE.txt) はフォントへのOFL-1.1の適用とソフトウェアへの同梱を明示しています。さらに [Utilities ページ](https://unifoundry.com/unifont/unifont-utilities.html) が配布する18.0.01アーカイブを取得して、以下を確認しました。

- `font/precompiled/unifont-18.0.01.hex` と同梱gzipを展開したデータはbyte単位で一致。非圧縮HEXのSHA-256は `2ab7801c809b76541a10b25858c1a962bdb650eee56b4c7df1f325b56e3955ee`。
- 同じリリースの標準BDFに実際の著作権表示（1998–2026、Roman Czyborra、Paul Hardy、Qianqian Fang ほか）があり、`licenses/UNIFONT-18.0.01-NOTICE.txt` にヘッダーを保存。
- 同リリースの `OFL-1.1.txt` を `licenses/UNIFONT-OFL-1.1.txt` に保存。確認したBDFヘッダーとOFL本文にはReserved Font Nameの指定がありません。共通LICENSE内の記入例を、このフォントの権利者や名前の指定とは扱いません。
- 元データと生成ROMのハッシュも既存の `assets/UNIFONT-LICENSE.txt` の記録と一致。ライセンス通知とROM変換の説明を同ファイルに追記し、GUIへ埋め込む通知全文にも収録。

これにより、前回記載したUnifontの版別権利表示の未確認事項は解消しました。フォントと変換ROMのOFL条件を保持し、CPT32自作部分にはMITを適用します。アーカイブのURLとSHA-256は版別通知に記録しています。

## 配布時の注意

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

2026-10-25: 共通ROMの JIS X 0208 収録字形は Jiskan16 に差し替えた。同梱Unifontの元HEXは未変更。その他の字形はUnifontを維持し、混合ROMはOFL-1.1で配布する。Jiskan16単体のBDFはパブリックドメインで、上流の表示を保存する。再生成コマンドは `doc/spec_VDP.md` を参照。
