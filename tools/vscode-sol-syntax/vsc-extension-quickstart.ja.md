---
lang: ja
translation_of: vsc-extension-quickstart.md
---
# VS Code 拡張機能へようこそ

## フォルダーの内容

* このフォルダーには、拡張機能に必要なすべてのファイルが含まれています。
* `package.json` - 言語サポートを宣言し、拡張機能にコピーされた文法ファイルの場所を定義するマニフェストファイルです。
* `syntaxes/sol.tmLanguage.json` - トークン化に使用される Text mate 文法ファイルです。
* `language-configuration.json` - コメントと括弧に使用するトークンを定義する言語設定です。

## すぐに使い始める

* `language-configuration.json` の言語設定が正確であることを確認します。
* `F5` を押して、拡張機能を読み込んだ新しいウィンドウを開きます。
* ファイル名の接尾辞が言語に一致する新しいファイルを作成します。
* 構文強調表示と、言語設定が機能していることを確認します。

## 変更を加える

* 上記のファイルを変更した後は、デバッグツールバーから拡張機能を再起動できます。
* 拡張機能に変更を読み込むには、VS Code ウィンドウを再読み込みすることもできます（Mac では `Ctrl+R` または `Cmd+R`）。

## 言語機能を追加する

* IntelliSense、ホバー、バリデーターなどの機能を追加するには、VS Code 拡張機能のドキュメント https://code.visualstudio.com/api/language-extensions/overview を参照してください。

## 拡張機能をインストールする

* Visual Studio Code で拡張機能を使い始めるには、`<user home>/.vscode/extensions` フォルダーにコピーして、Code を再起動します。
* 拡張機能を公開して共有する方法については、https://code.visualstudio.com/api/working-with-extensions/publishing-extension の公開に関する説明をお読みください。
