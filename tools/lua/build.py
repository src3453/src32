#!/usr/bin/env python3
"""外部の Lua 5.4 ソースを pkcl と Newlib で組み、Foundation の lua.wax を作る。

Lua は MIT ライセンスだがリポジトリには含めず、Newlib と同じく取得済みのソースを指定する。
パッチは当てない。JIT は含まない（Lua 本体のインタプリタと REPL だけ）。

    python tools/lua/build.py --lua-source DIR --newlib-source DIR \\
        --fnd-wal root/lib/fnd.1.wal --out target/foundation-lua
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
# luac.c は別アプリ。lua.c が REPL とスクリプト実行を持つ。
EXCLUDED = {"luac.c"}


def lua_version(source: Path) -> str:
    text = (source / "src/lua.h").read_text(encoding="utf-8")
    parts = [re.search(rf"#define LUA_VERSION_{name}\s+\"(\d+)\"", text) for name in ("MAJOR", "MINOR")]
    release = re.search(r'#define LUA_VERSION_RELEASE\s+"(\d+)"', text)
    if not all(parts) or not release:
        raise SystemExit(f"{source}: not a Lua source tree (src/lua.h)")
    return f"{parts[0][1]}.{parts[1][1]}.{release[1]}"


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--lua-source", type=Path, required=True)
    p.add_argument("--newlib-source", type=Path, required=True)
    p.add_argument("--fnd-wal", type=Path, required=True)
    p.add_argument("--out", type=Path, default=ROOT / "target/foundation-lua")
    p.add_argument("--clang")
    p.add_argument("--cache", type=Path, help="Newlib IR cache")
    p.add_argument("-O", dest="optimization", default="-O1", choices=["-O0", "-O1", "-O2", "-Os", "-Oz"])
    p.add_argument("-j", "--jobs", type=int, default=4)
    args = p.parse_args(argv)
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    source = args.lua_source.resolve()
    version = lua_version(source)
    if not version.startswith("5.4."):
        raise SystemExit(f"Lua 5.4 only (got {version})")
    pkcl = ROOT / "foundation/lib/pkcl"
    sources = [s for s in sorted((source / "src").glob("*.c")) if s.name not in EXCLUDED]
    sources += [*sorted(pkcl.glob("*.c")), pkcl / "newlib/syscalls.c", *sorted((pkcl / "posix").glob("*.c"))]
    command = [sys.executable, str(ROOT / "tools/llvm-src32/driver.py"), *map(str, sources),
               args.optimization, "-j", str(args.jobs),
               "-I", str(ROOT / "foundation/include/pkcl-posix"),
               "-I", str(ROOT / "foundation/include"), "-I", str(ROOT / "sdk/cpt32/include"),
               "-D", "PKCL_LIBC",
               "-D", "LUA_USE_JUMPTABLE=0",  # computed goto (indirectbr) is not supported by the backend
               "--newlib-source", str(args.newlib_source.resolve()), "--newlib-profile", "posix",
               "--newlib-cache", str((args.cache or args.out / "newlib-cache").resolve()),
               "--entry", "pkcl_start", "--wax3", "--pie", "--page-align", "--image-id", "lua",
               "--user-stack-size", "0x20000", "--heap-hint", "0x400000",
               "--import-wal", str(args.fnd_wal.resolve()), "-o", str(args.out / "lua.wax")]
    if args.clang:
        command += ["--clang", args.clang]
    result = subprocess.run(command)
    if result.returncode:
        raise SystemExit(result.returncode)
    (args.out / "sources.json").write_text(json.dumps({
        "lua": str(source), "version": version, "newlib": str(args.newlib_source.resolve()),
    }, indent=2) + "\n", encoding="utf-8")
    print(f"lua {version}: {args.out / 'lua.wax'}")


if __name__ == "__main__":
    main()
