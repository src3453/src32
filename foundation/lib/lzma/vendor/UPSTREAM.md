# LZMA SDK 26.05

Igor Pavlov の LZMA SDK 26.05 (2026-10-21) から必要な C ファイルだけを変更せず収録。

- 公式説明・public domain 宣言: https://www.7-zip.org/sdk.html
- 配布アーカイブ: https://github.com/ip7z/7zip/releases/download/26.05/lzma2605.7z
- アーカイブ SHA-256: `668fda469b7f584f246008cf28359c66ef98d65fafb3b22a52672eebe6970c28`
- 原文の説明と許諾: `lzma-sdk.txt` (アーカイブの `DOC/lzma-sdk.txt`)

`LzmaEnc.c`, `LzmaDec.c`, `LzFind.c` とそれらのヘッダーはゲスト用。
`CpuArch.c`, `7zWindows.h` はホスト相互運用テスト用で、WAL にはリンクしない。
7z/XZ/LZMA2、アセンブリ、スレッド、ファイル I/O は組み込まない。
親ディレクトリのアダプターとビルドツールは CPT32 のコード。
