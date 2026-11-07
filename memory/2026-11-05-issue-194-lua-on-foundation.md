# Lua 5.4 を Foundation で動かす (#194)、WAL 化の壁 (#195)

- Lua は MIT だがコミットしない。`tools/lua/build.py --lua-source DIR --newlib-source DIR --fnd-wal root/lib/fnd.1.wal`（Newlib posix profile + pkcl の静的リンク、WAX v3 PIE）。検証は Lua 5.4.9、Newlib 4.4.0.20231231、`clang-20` + `llvmlite==0.46.*`。
- 取得: このコンテナは lua.org / sourceware / github への HTTPS が 403。Ubuntu アーカイブの `lua5.4_5.4.9.orig.tar.gz`、`newlib_4.4.0.20231231.orig.tar.gz`（`archive.ubuntu.com/ubuntu/pool/universe/{l/lua5.4,n/newlib}/`）は取れる。
- 唯一のバックエンド側ギャップは `ljumptab.h` の computed goto（`indirectbr`）。`-DLUA_USE_JUMPTABLE=0` で回避（パッチ不要）。setjmp、strtod、`%.14g`、libm、コルーチン、io は追加実装なしで動く。
- `lua.wax` は -O1 で約 2.1 MiB（staging 3 MiB に収まる）。ビルドは -j8 で約 2.5 分、テストの実行は 1 本 75〜105 秒。
- PCG 画面ダンプではタブが空白に展開されるので、期待文字列は空白を正規化して比べる（`test_lua.py`）。テストは `SRC32_LUA_WAX` で組み済み WAX を渡すとビルドを省ける。
- WAL 化は不可（#195）: WAL はアドレスを含む静的データを持てず（`@boxmt` で停止）、`fnptr` も未対応。方式は WAL への load-time relocation 追加／ソース変換／静的ライブラリのどれかで、オーナーの判断待ち。
