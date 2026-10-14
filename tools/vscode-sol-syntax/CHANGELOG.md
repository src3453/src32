# Change Log

All notable changes to the "vscode-sol-syntax" extension will be documented in this file.

Check [Keep a Changelog](http://keepachangelog.com/) for recommendations on how to structure this file.

## [Unreleased]
- Support `!namespace` blocks and aliased `!include` namespaces in compiler diagnostics.
- Support UTF-8 string literals by default and `u16"..."` UTF-16BE literals in the bundled language server.
- Highlight the `u16` string prefix.

- Updated the bundled language server and compiler to accept assignments to function arguments.
- Initial release