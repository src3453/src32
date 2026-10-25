---
lang: ja
translation_of: ../spec_CPT32PKG.md
---
# CPT32PKG カートリッジパッケージフォーマット

CPT32PKG は SRC32 ゲストプログラム用の完全な読み取り専用 ZIP パッケージです。パッケージには IDC ディスクイメージおよび既存の WASM 拡張カードパッケージも宣言できます。`.cpt32card` フォーマットは独立した WASM カードフォーマットのままであり、CPT32PKG に置き換えられることはありません。

## アーカイブのレイアウトと制限

すべてのパッケージにはルートの `manifest.yaml` があります。ZIP メンバーのパスはスラッシュを使用し、相対パスかつ空でないものとし、空の構成要素、`.`、`..`、バックスラッシュ、NUL、ドライブ接頭辞／コロンを含んではなりません。ZIP ディレクトリエントリーは構造として受け入れますが、リソースには数えません。正規化後の名前が重複する場合は拒否します（大文字小文字を区別）。ディレクトリ名末尾のスラッシュは衝突検査時に削除されます。通常ファイルでもディレクトリでもないエントリー、および Stored と Deflated 以外の圧縮方式は拒否します。ルートの manifest は通常ファイルでなければなりません。

manifest は 1 MiB に制限されます。参照されていないアセットを含め、各通常 ZIP メンバーは展開後 64 MiB に制限されます。合計サイズの上限はありません。ローダーはアーカイブ構造とメンバーメタデータを検証した後、manifest とそこから参照されるファイルのみを展開します。その他の安全なファイルは不透明なまま保持され、先行して展開されません。リソースは上限付きアロケーションに読み込まれ、実際の展開サイズは ZIP が宣言するサイズと正確に一致しなければなりません。ローダーはアーカイブから展開したり、アーカイブへ書き込んだりしません。

## Manifest

manifest は UTF-8 YAML です。すべてのスキーマ階層で未知のフィールドはエラーです。必須フィールドと正確に受け入れられる値:

```yaml
format: cpt32pkg
format_version: 1
id: 00000000-0000-4000-8000-000000000001
title: Example CPT32 software
system:
  architecture: SRC32
entrypoint:
  type: program
  path: program.bin
disks:
  - slot: 0
    type: hdd
    path: disks/system.img
    mode: read_write
plugins:
  - slot: 0
    path: plugins/example.cpt32card
```

`format` は `cpt32pkg` でなければならず、`format_version` は整数 `1` でなければなりません。`id` は `8-4-4-4-12` の16進形式による正準的な小文字 UUID であり、パッケージのユーザーデータに使う安定した識別子です。`title` は空であってはなりません。`system.architecture` は `SRC32` です。`entrypoint.type` は `program` です。その path はアドレス 0 にロードされる空でない raw ゲストバイナリを指定し、BIOS ブートイメージを指定してはなりません。また、そのサイズは 16 MiB の Main RAM に収まらなければなりません。disk と plugin の配列は省略可能です。配列を省略した場合、デバイスはマウントされません。

各 disk は 0 から 3 までの一意な `slot`、`fdd` または `hdd` の `type`、安全なアーカイブ `path`、および必須の `read_only` または `read_write` の `mode` を持ちます。イメージは空でなく、バイトサイズが 512 で割り切れ、32-bit のセクター容量を超えてはなりません。読み取り専用メディアは変更できません。書き込み可能なパッケージメディアは、カートリッジごとの外部 COW サイドカーを使用します。ZIP パッケージは変更されません。

各 plugin は 0 から 7 までの一意な `slot` と、既存のネストされた `.cpt32card` パッケージを指定する安全な `path` を持ちます。ネストされたパッケージには、既存の WASM カード検証およびアセットインターフェースが適用されます。

entrypoint、各 disk、各 plugin はそれぞれ異なる path を参照しなければならず、いずれも `manifest.yaml` を参照してはなりません。参照先は通常ファイルとして解決されなければなりません。参照されない安全なファイルは不透明な同梱アセットです。ゲストソフトウェアは宣言済みディスクイメージを通じて同梱データにアクセスし、plugin は既存の読み取り専用アセット API を使用します。CPT32PKG はゲストアセット ABI を追加しません。

## パッケージ例と作成

レイアウト例:

```text
manifest.yaml
program.bin
disks/system.img
plugins/example.cpt32card
assets/readme.txt
```

manifest と参照ファイルを含むディレクトリから、次のようにアーカイブを作成します:

```sh
zip -X -q example.cpt32pkg manifest.yaml program.bin disks/system.img plugins/example.cpt32card assets/readme.txt
```

不透明なアセットは任意であり、manifest に列挙する必要はありません。ローダーはパッケージメンバーを展開しません。

Python Launcherの「CPT32PKG保存…」でも、選択中のrawプログラム、有効なディスクと拡張カードを同梱できます。タイトルには保存先のファイル名（拡張子を除く）を使い、保存ごとに新しいUUIDを生成します。ディスクのスロット・種類・読み取り専用属性とカードのスロットを維持します。CPU・メモリ・BIOS・フォント・音楽・ディスク速度などのホスト設定は本形式の対象外です。WAX・sol・既存パッケージの変換やBIOSディスク起動の保存には対応しません。詳細は `tools/cpt32-launcher/README.md` を参照してください。

## 起動

通常の CPT32 CLI を使ってパッケージを起動します:

```sh
cpt32 example.cpt32pkg
```

`.cpt32pkg` 拡張子は大文字小文字を区別せず認識されます。その他のゲストパスは従来どおり raw-binary 起動動作を維持します。
