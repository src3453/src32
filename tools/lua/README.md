# Lua 5.4 を Foundation で動かす

Lua（MIT）はリポジトリに含めません。公式 tarball などで取得したソースと外部 Newlib を指定してビルドします（Newlib・DOOMGeneric と同じ方式）。パッチは当てません。JIT はありません。

```sh
# 1. Foundation を一度ビルドして fnd.1.wal を得る
python foundation/boot/build.py target/boot
# 2. lua.wax を作る（Lua 5.4.x と Newlib 4.4 のソースのパス）
python tools/lua/build.py --lua-source /path/to/lua-5.4.9 --newlib-source /path/to/newlib-4.4.0.20231231 \
    --fnd-wal target/boot/root/lib/fnd.1.wal --out target/foundation-lua
# 3. 起動ディスクの /bin に取り込んで起動する（--bin-dir は直下の *.wax を /bin に足す）
python foundation/run.py --disk --bin-dir target/foundation-lua
```

シェルから `lua`（REPL）、`lua /script.lua`（スクリプト）で使います。`lua.wax` は約 2.1 MiB で、spawn の 3 MiB staging に収まります。

- ゲストテスト: `SRC32_LUA_SOURCE=... SRC32_NEWLIB_SOURCE=... python -m unittest discover -s foundation/tests/lua -v`（`SRC32_LUA_WAX` で組み済み WAX を渡すとビルドを省く）。
- `-DLUA_USE_JUMPTABLE=0` が必要です（バックエンドは computed goto の `indirectbr` 未対応）。
- 共有ライブラリ（WAL）化は #195。
