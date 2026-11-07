# PR #196 の Lua を Windows で実地確認 (#194)

- 環境: Windows、Python 3.15 + llvmlite 0.50（LLVM 22）、`C:/Program Files/LLVM/bin/clang.exe` 20.1.8、Newlib は `target/busybox-work/newlib-4.4.0.20231231`、Lua は lua.org の 5.4.9（`target/lua-work/`）。Clang と llvmlite のメジャー版はずれているが -O1 のビルドは通る。`run.py --disk --lua --build-only` は約 2.5 分、`lua.wax` は 2,302,496 byte。
- `foundation/tests/lua` は `cached`、`FOUNDATION_TEST_CPU=pipeline`、`FND_MM=nommu` の 3 構成とも 2 件通過（`SRC32_LUA_WAX` で上の wax を渡した）。
- `run.py` のディスクは停止タイマーなしなので、testbench では `--allow-running` を付ける。BIOS 起動はプログラムに 4 byte のゼロ（`run/no-ram-program.bin`）を渡し、config の `bios:` を使う。ディスクは書き換わるのでコピーで試す。
- Git Bash では `MSYS_NO_PATHCONV=1` がないと `fs2.py` の `/bin` などがホストパスに変換される（`BAD_PATH`）。
- `string.rep` の連結を積むメモリ枯渇テストは、16 MiB RAM だと保持約 5 MiB で `not enough memory` になり、pcall で回復して動き続ける。`heap_hint` は初期予約で上限ではない（`mem_grow` で伸びる）。64 MiB では 7,000 フレームでも枯渇に届かず、枯渇点の数値は未測定。
- エラーメッセージの progname は `/bin/lua.wax:` になる（argv[0] がパス）。
