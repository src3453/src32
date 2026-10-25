#!/usr/bin/env python3
"""Build external DOOMGeneric + Newlib as a Foundation WAX (no downloads)."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
REVISION = "e257a09527062bfb986b6e5e0780839b6cbd5b71"


def prepare(source, out):
    """Keep the checkout untouched; adapt the upstream quit path in a build copy."""
    source = source / "doomgeneric"
    makefile = (source / "Makefile").read_text(encoding="utf-8")
    match = re.search(r"^SRC_DOOM = (.+)$", makefile, re.M)
    if not match:
        raise ValueError("DOOMGeneric Makefile has no SRC_DOOM list")
    names = [Path(obj).with_suffix(".c").name for obj in match[1].split()
             if obj not in ("doomgeneric_xlib.o", "w_file_stdc.o")]
    out.mkdir(parents=True, exist_ok=True)
    system = (source / "i_system.c").read_text(encoding="utf-8")
    # Upstream's non-ORIGCODE I_Quit runs hooks but never exits. Also omit the
    # desktop popup: errors still go through stderr and exit(-1).
    marker = "#if ORIGCODE\n    SDL_Quit();\n\n    exit(0);"
    if system.count(marker) != 1 or system.count("ZenityErrorBox(msgbuf);") != 1:
        raise ValueError("unsupported DOOMGeneric i_system.c; review quit/error adaptation")
    system = system.replace(marker, "    exit(0);\n\n" + marker)
    system = system.replace("ZenityErrorBox(msgbuf);", "(void)msgbuf;")
    adapted = out / "i_system.c"
    adapted.write_text(system, encoding="utf-8")
    return [adapted if name == "i_system.c" else source / name for name in names]


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--doomgeneric-source", type=Path, required=True)
    p.add_argument("--newlib-source", type=Path, required=True)
    p.add_argument("--fnd-wal", type=Path, required=True)
    p.add_argument("--out", type=Path, default=ROOT / "target/foundation-doom")
    p.add_argument("--clang")
    p.add_argument("--cache", type=Path)
    p.add_argument("-j", "--jobs", type=int, default=4)
    args = p.parse_args(argv)
    args.out = args.out.resolve()
    source = args.doomgeneric_source.resolve()
    sources = prepare(source, args.out / "adapted")
    pkcl = ROOT / "foundation/lib/pkcl"
    sources += [HERE / "doomgeneric_foundation.c", HERE / "w_file_foundation.c", *sorted(pkcl.glob("*.c")),
                pkcl / "newlib/syscalls.c", *sorted((pkcl / "posix").glob("*.c"))]
    command = [sys.executable, str(ROOT / "tools/llvm-src32/driver.py"),
               *map(str, sources), "-O1", "-j", str(args.jobs),
               "-I", str(source / "doomgeneric"),
               "-I", str(ROOT / "foundation/include/pkcl-posix"),
               "-I", str(ROOT / "foundation/include"), "-I", str(ROOT / "sdk/cpt32/include"),
               "-D", "PKCL_LIBC", "-D", "_DEFAULT_SOURCE", "-D", "NORMALUNIX",
               "-D", "CMAP256", "-D", "DOOMGENERIC_RESX=320", "-D", "DOOMGENERIC_RESY=200",
               "--newlib-source", str(args.newlib_source.resolve()), "--newlib-profile", "posix",
               "--newlib-cache", str((args.cache or args.out / "newlib-cache").resolve()),
               "--entry", "pkcl_start", "--wax3", "--pie", "--page-align", "--image-id", "doom",
               "--user-stack-size", "0x20000", "--heap-hint", "0x1800000",
               "--import-wal", str(args.fnd_wal.resolve()), "-o", str(args.out / "doom.wax")]
    if args.clang:
        command += ["--clang", args.clang]
    result = subprocess.run(command)
    if result.returncode:
        raise SystemExit(result.returncode)
    rev = subprocess.run(["git", "-C", str(source), "rev-parse", "HEAD"],
                         capture_output=True, text=True)
    (args.out / "sources.json").write_text(json.dumps({
        "doomgeneric": str(source), "revision": rev.stdout.strip(),
        "tested_revision": REVISION, "newlib": str(args.newlib_source.resolve()),
    }, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
