# Change Log

All notable changes to the "vscode-sol-syntax" extension will be documented in this file.

Check [Keep a Changelog](http://keepachangelog.com/) for recommendations on how to structure this file.

## [Unreleased]
- Add declaration, definition, hover, document highlight, references and workspace symbol providers so `Go to Declaration`/`Go to Definition`, `Find All References`, `Go to Symbol in Workspace` and hover work, including through `!include`d files and `namespace.name` qualifiers.
- Add a document symbol provider so namespaces, functions, constants, variables, locals and macros appear in the outline.
- Mark namespace blocks, aliased `!include` namespaces and `namespace.name` qualifiers as namespaces in semantic highlighting.
- Report exact token columns after string literals and punctuation, and stop emitting semantic tokens for punctuation and raw `!asm` bodies.
- Support `!namespace` blocks and aliased `!include` namespaces in compiler diagnostics.
- Support UTF-8 string literals by default and `u16"..."` UTF-16BE literals in the bundled language server.
- Highlight the `u16` string prefix.

- Updated the bundled language server and compiler to accept assignments to function arguments.
- Initial release