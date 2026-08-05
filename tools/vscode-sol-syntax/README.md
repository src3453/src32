# Sol Syntax Highlighting for VS Code

This extension adds TextMate-based syntax highlighting for the Sol language.

Build & package

- Using npx (no global install required):
  - npx vsce package

- Or install vsce globally and run:
  - npm i -g vsce
  - vsce package

After packaging, install the produced .vsix in VS Code (Extensions: Install from VSIX...).

Notes

- This extension is a minimal TextMate grammar. Adjust patterns in syntaxes/sol.tmLanguage.json to match the current Sol spec.
