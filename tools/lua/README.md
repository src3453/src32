# Lua 5.5 を Foundation で動かす

Lua（MIT）はリポジトリに含めません。公式 tarball などで取得したソースと外部 Newlib を指定してビルドします（Newlib・DOOMGeneric と同じ方式）。パッチは当てません。JIT はありません。

一番簡単なのは `foundation/run.py` のオプションです（標準のディスクには入りません）。fnd.1.wal の構築、Lua のビルド、起動ディスクの `/bin/lua.wax` への取り込みまでを自動で行います。

```sh
python foundation/run.py --disk --lua --lua-source /path/to/lua-5.5.1 --newlib-source /path/to/newlib-4.4.0.20231231
# 環境変数 SRC32_LUA_SOURCE / SRC32_NEWLIB_SOURCE があれば、--lua だけでよい
```

手順を分けるときは次のとおりです。

```sh
python foundation/boot/build.py target/boot
python tools/lua/build.py --lua-source /path/to/lua-5.5.1 --newlib-source /path/to/newlib-4.4.0.20231231 \
    --fnd-wal target/boot/root/lib/fnd.1.wal --out target/foundation-lua
python foundation/run.py --disk --bin-dir target/foundation-lua   # --bin-dir は直下の *.wax を /bin に足す
```

シェルから `lua`（REPL）、`lua /script.lua`（スクリプト）で使います。`lua.wax` は約 2.3 MiB で、spawn の 3 MiB staging に収まります。

- ゲストテスト: `SRC32_LUA_SOURCE=... SRC32_NEWLIB_SOURCE=... python -m unittest discover -s foundation/tests/lua -v`（`SRC32_LUA_WAX` で組み済み WAX を渡すとビルドを省く）。
- `-DLUA_USE_JUMPTABLE=0` が必要です（バックエンドは computed goto の `indirectbr` 未対応）。
- 共有ライブラリ（WAL）化は #195。
