#!/usr/bin/env python3
"""Build the C BIOS ROM image (default output: assets/bios.rom).

The image is raw, starts at the ROM base 0xFF000000 and must fit the 64 KiB
BIOS window. Writable data lives in the BIOS work area at 0x000F0000 and the
stack grows down from 0x00100000 (doc/spec_BIOS.md).
"""
import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ROM_BASE = 0xFF000000
ROM_SIZE = 0x10000
DATA_ORIGIN = 0x000F0000
STACK_TOP = 0x00100000
SOURCES = [
    ROOT / "bios" / "main.c",
    ROOT / "sdk" / "cpt32" / "src" / "pcg.c",
    ROOT / "sdk" / "cpt32" / "src" / "idc.c",
    ROOT / "sdk" / "cpt32" / "src" / "irqc.c",
]


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("-o", "--output", type=Path, default=ROOT / "assets" / "bios.rom")
    parser.add_argument("-O", dest="optimization", choices="012", default="1")
    parser.add_argument("--clang", help="Clang executable passed to the LLVM/SRC32 driver")
    args = parser.parse_args(argv)
    with tempfile.TemporaryDirectory() as work:
        image = Path(work) / "bios.bin"
        command = [sys.executable, str(ROOT / "tools" / "llvm-src32" / "driver.py"), *map(str, SOURCES),
                   "-I", str(ROOT / "sdk" / "cpt32" / "include"), f"-O{args.optimization}",
                   "--origin", hex(ROM_BASE), "--data-origin", hex(DATA_ORIGIN),
                   "--stack-top", hex(STACK_TOP), "-o", str(image)]
        if args.clang:
            command += ["--clang", args.clang]
        subprocess.run(command, check=True, stdin=subprocess.DEVNULL)
        size = image.stat().st_size
        if size > ROM_SIZE:
            print(f"bios: image is {size} bytes, larger than the {ROM_SIZE}-byte ROM window", file=sys.stderr)
            return 1
        args.output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(image, args.output)
    print(f"bios: wrote {args.output} ({size} bytes of {ROM_SIZE})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
