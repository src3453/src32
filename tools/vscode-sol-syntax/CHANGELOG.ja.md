---
lang: ja
translation_of: CHANGELOG.md
---
# 変更履歴

`vscode-sol-syntax` 拡張機能の重要な変更を記録します。

構成の推奨事項については、[Keep a Changelog](http://keepachangelog.com/) を参照してください。

## [未リリース]
- document symbol provider を追加し、名前空間・関数・定数・変数・ローカル変数・マクロをアウトラインに表示できるようにしました。
- 名前空間ブロック、別名付き `!include` namespace、`namespace.name` 修飾子をセマンティックハイライトで名前空間として扱うようにしました。
- 文字列リテラルや記号の後ろでもトークンの桁位置を正しく報告し、記号と `!asm` ブロック本体にはセマンティックトークンを出力しないようにしました。
- コンパイラ診断で `!namespace` ブロックと、別名付き `!include` namespace をサポート。
- 同梱の language server で、既定の UTF-8 文字列リテラルと `u16"..."` UTF-16BE 文字列リテラルをサポート。
- `u16` 文字列プレフィックスをシンタックスハイライト。

- 同梱の language server と compiler を更新し、関数引数への代入を受け付けるよう変更。
- 初回リリース。
