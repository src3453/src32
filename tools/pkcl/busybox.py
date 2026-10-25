#!/usr/bin/env python3
"""Build BusyBox for Foundation with pkcl and Newlib (doc/spec_pkcl.md §8).

BusyBox is GPL-2.0 and not part of this repository: pass the root of a source
tree you obtained yourself (and the same for Newlib, see
examples/newlib-c/README.md).

    python tools/pkcl/busybox.py --busybox-source DIR --newlib-source DIR \\
        --fnd-wal root/lib/fnd.1.wal --out build/busybox

Steps: (1) configure BusyBox out of tree with tools/pkcl/busybox.config and run
BusyBox's own build once with the host compiler, which generates its headers
and tells us exactly which sources it compiles and with which flags; (2) compile
each of those sources to LLVM IR with tools/llvm-src32 (-c); (3) link the IR,
pkcl and Newlib into busybox.wax. Needs the host's gcc, make and bison.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DRIVER = ROOT / "tools/llvm-src32/driver.py"
PKCL = ROOT / "foundation/lib/pkcl"
# Slot 5 of the user slots (spec_Foundation.md §5.2) with a 64 KiB user stack.
LAYOUT = ["--origin", "0x800000", "--stack-top", "0xC00000", "--user-stack-size", "0x20000"]


def run(command, cwd=None, log=None, check=True):
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True, errors="replace")
    if log:
        Path(log).write_text(result.stdout + result.stderr, encoding="utf-8")
    if check and result.returncode:
        raise SystemExit(f"{' '.join(map(str, command[:3]))}…: exit {result.returncode}\n"
                         f"{(result.stderr or result.stdout)[-3000:]}")
    return result


def configure(args, build: Path):
    build.mkdir(parents=True, exist_ok=True)
    make = ["make", "-C", str(args.busybox_source), f"O={build}"]
    run([*make, "allnoconfig"], log=build / "allnoconfig.log")
    wanted = {}
    for line in args.config.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip() if not line.startswith("CONFIG_") else line.strip()
        if "=" in line:
            key, value = line.split("=", 1)
            wanted[key] = value
    lines, seen = [], set()
    for line in (build / ".config").read_text(encoding="utf-8").splitlines():
        match = re.match(r"# (CONFIG_\w+) is not set", line) or re.match(r"(CONFIG_\w+)=", line)
        if match and match[1] in wanted:
            key = match[1]
            lines.append(f"# {key} is not set" if wanted[key] == "n" else f"{key}={wanted[key]}")
            seen.add(key)
        else:
            lines.append(line)
    lines += [f"{k}={v}" for k, v in wanted.items() if k not in seen and v != "n"]
    (build / ".config").write_text("\n".join(lines) + "\n", encoding="utf-8")
    run(["sh", "-c", f"yes '' | timeout 300 {shlex.join(make)} oldconfig"], log=build / "oldconfig.log")
    # Hard dependencies can switch an option back off; say so rather than build something else.
    final = (build / ".config").read_text(encoding="utf-8")
    for key, value in wanted.items():
        if value != "n" and f"{key}={value}" not in final:
            print(f"warning: {key}={value} was not accepted by BusyBox's dependency rules", file=sys.stderr)
    # BusyBox 1.36 tests MAKEFLAGS for any 's', including GNU make 4.4's
    # --jobserver-auth. Override quiet so V=1 still records compile commands.
    run([*make, "V=1", "quiet=", f"-j{args.jobs}"], log=build / "host-build.log", check=False)
    if not (build / "include/autoconf.h").is_file():
        raise SystemExit(f"BusyBox's host build failed; see {build / 'host-build.log'}")


def target_sources(build: Path):
    """(source, flags) of every translation unit linked into busybox, from the host build's log."""
    commands = {}
    for line in (build / "host-build.log").read_text(encoding="utf-8", errors="replace").splitlines():
        line = line.strip()
        if not re.match(r"(gcc|cc) .* -c ", line) or "echo " in line[:8]:
            continue
        words = shlex.split(line)
        if "-o" not in words:
            continue
        obj = words[words.index("-o") + 1]
        source = words[-1]
        # Assembly and the host-architecture variants (SHA hardware acceleration) are not for this target.
        if not source.endswith(".c") or re.search(r"_(x86|arm|aarch)", Path(source).name):
            continue
        flags = []
        index = 1
        while index < len(words) - 1:
            word = words[index]
            if word in ("-include", "-I", "-D", "-o"):
                flags.append((word, words[index + 1]))
                index += 2
                continue
            for prefix in ("-I", "-D"):
                if word.startswith(prefix) and len(word) > 2:
                    flags.append((prefix, word[2:]))
            index += 1
        commands[obj] = (source, flags)
    # Objects that end up in the final link: members of each lib.a, plus applets.o.
    members = {"applets/applets.o"}
    for archive in sorted(build.rglob("lib.a")):
        listing = run(["ar", "t", str(archive)], check=False).stdout.split()
        members |= {str(archive.parent.relative_to(build) / member) for member in listing}
    chosen = {obj: commands[obj] for obj in members if obj in commands}
    missing = sorted(members - set(chosen))
    if missing:
        print("warning: no compile command for", ", ".join(missing), file=sys.stderr)
    return chosen


def driver_args(args):
    return ["--newlib-source", str(args.newlib_source), "--newlib-profile", "posix",
            *(["--newlib-cache", str(args.cache)] if args.cache else []),
            *(["--clang", args.clang] if args.clang else [])]


EMPTY_ASM = re.compile(r'^\s*(?:tail )?call void asm sideeffect "",.*\n', re.M)


def strip_empty_asm(ir: Path):
    """BusyBox の barrier() が出す空のインラインアセンブリを消す（バックエンドは inline asm を持たず、
    SRC32 は 1 命令ずつ順に実行するのでコンパイラバリアは何もしなくてよい）。"""
    text = ir.read_text()
    stripped = EMPTY_ASM.sub("", text)
    if stripped != text:
        ir.write_text(stripped)


def compile_all(args, build: Path, out: Path):
    out.mkdir(parents=True, exist_ok=True)
    units = target_sources(build)
    if args.only:
        units = {obj: unit for obj, unit in units.items() if re.search(args.only, obj)}
    failures = {}

    def compile_one(item):
        obj, (source, flags) = item
        ir = out / (obj.replace("/", "-")[:-2] + ".ll")
        command = [sys.executable, str(DRIVER), "-c", source, args.optimization,
                   "-D", "__clang_major__=0",  # インラインアセンブリを使う Clang 向け分岐を避ける
                   "-D", "BB_GLOBAL_CONST=",   # ptr_to_globals を rodata に置かない（U モードでは書込みフォルトになる）
                   "-I", str(ROOT / "foundation/include/pkcl-posix"), *driver_args(args), "-o", str(ir)]
        for kind, value in flags:
            if kind == "-I":
                command += ["-I", str((build / value).resolve())]
            elif kind == "-D":
                command += ["-D", value]
            elif kind == "-include":
                command += ["--include", str((build / value).resolve())]
        result = run(command, cwd=build, check=False)
        if result.returncode:
            failures[obj] = (result.stderr or result.stdout).strip()
            return None
        strip_empty_asm(ir)
        return ir

    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        irs = [ir for ir in pool.map(compile_one, sorted(units.items())) if ir]
    summary = {}
    for obj, text in sorted(failures.items()):
        first = [m[1] for m in (re.search(r":\d+:\d+: (?:fatal )?error: (.*)", line) for line in text.splitlines()) if m]
        key = first[0] if first else text.splitlines()[0][:120]
        summary.setdefault(key, []).append(obj)
    for key, objs in sorted(summary.items(), key=lambda item: -len(item[1])):
        print(f"{len(objs):4d} {key}  [{', '.join(objs[:3])}{'…' if len(objs) > 3 else ''}]", file=sys.stderr)
    print(f"compiled {len(irs)} of {len(units)} translation units")
    return irs, failures


def link(args, irs, out: Path):
    sources = [PKCL / name for name in ("errno.c", "fd.c", "fs.c", "crt.c", "newlib/syscalls.c")]
    extra = [*sorted(PKCL.glob("posix/*.c")), HERE / "busybox_stubs.c"]
    command = [sys.executable, str(DRIVER), *map(str, irs), *map(str, sources), *map(str, extra), args.optimization,
               "-I", str(ROOT / "foundation/include"), "-I", str(ROOT / "sdk/cpt32/include"),
               "-I", str(ROOT / "foundation/include/pkcl-posix"), "-D", "PKCL_LIBC",
               *driver_args(args), "-j", str(args.jobs), "--entry", "pkcl_start", "--wax3", "--page-align",
               "--image-id", "busybox", "--import-wal", str(args.fnd_wal), *LAYOUT, "-o", str(out / "busybox.wax")]
    result = run(command, check=False)
    print((result.stdout + result.stderr).strip()[-1500:])
    return result.returncode


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--busybox-source", type=Path, required=True)
    parser.add_argument("--newlib-source", type=Path, required=True)
    parser.add_argument("--fnd-wal", type=Path, required=True, help="/lib/fnd.1.wal built by foundation/boot/build.py")
    parser.add_argument("--out", type=Path, default=Path("build/busybox"))
    parser.add_argument("--config", type=Path, default=HERE / "busybox.config")
    parser.add_argument("--clang")
    parser.add_argument("--cache", type=Path, help="Newlib IR cache (default: tools/llvm-src32's)")
    parser.add_argument("-O", dest="optimization", default="-Os", choices=["-O0", "-O1", "-O2", "-Os", "-Oz"])
    parser.add_argument("-j", "--jobs", type=int, default=os.cpu_count() or 1)
    parser.add_argument("--only", help="regular expression: compile only the matching objects (debugging)")
    parser.add_argument("--stage", choices=["configure", "compile", "link"], default="link")
    args = parser.parse_args(argv)
    args.busybox_source = args.busybox_source.resolve()
    args.out = args.out.resolve()
    build = args.out / "bb"
    if not (build / "include/autoconf.h").is_file():
        configure(args, build)
    if args.stage == "configure":
        return 0
    irs, failures = compile_all(args, build, args.out / "ir")
    if failures or args.stage == "compile":
        return 1 if failures else 0
    return link(args, irs, args.out)


if __name__ == "__main__":
    raise SystemExit(main())
