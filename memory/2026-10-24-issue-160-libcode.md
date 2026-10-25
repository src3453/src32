# libcode（文字コード変換 WAL）

- `foundation/lib/code`、image ID `code` 1.0、`/lib/code.1.wal`。API は `<foundation/code.h>`。owner の指示で **UTF-32BE を基準**にし、`cpt_code_<enc>_to_utf32be`／`cpt_code_utf32be_to_<enc>`（`utf8`、`utf8bom`、`utf16le`、`utf16be`、`utf32le`）の対にした。直接の `convert(from,to)` は作らない（他の符号化どうしは呼出側が経由する）。
- 戻り値・`*dst_size` は conde と同じ。`flags` で `CODE_REPLACE`（U+FFFD、maximal subpart）、`CODE_STRIP_BOM`、`CODE_ADD_BOM`。既定は不正入力でエラー（`CONDE_DATA`、末尾の途中切れは `CONDE_INPUT_EOF`）。
- BOM の扱い: `utf8bom` は出力で必ず付け、入力で読み飛ばす。明示的な UTF-16/32 の先頭 U+FEFF は文字として写す（Unicode の規定どおり）。`cpt_code_detect` は `FF FE 00 00` を UTF-32LE とする。
- 落とし穴: llvm-src32 は `llvm.uadd.sat` を持たない。32 bit のオーバーフロー判定を `a + b < a` や飽和風に書くと clang が intrinsic にするので、`uint64_t` で足して比較する。
- 検証: `CLANG=clang-20 python3 -m unittest foundation.tests.code.test_code`（ホストは Python の codecs と照合、`errors="replace"` の結果とも一致を見る。Foundation 上は `app.c`）。ディスクには `boot/build.py` が入れ、`tests/disk` が存在を確認する。
