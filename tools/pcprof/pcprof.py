#!/usr/bin/env python3
"""Per-function instruction profile for SRC32 programs (see README.md).

    python tools/pcprof/pcprof.py report PROFILE ASM [--top N] [--json]
    python tools/pcprof/pcprof.py symbols ASM [--grep TEXT]
    python tools/pcprof/pcprof.py foundation OUTDIR [--command "top -n 4"] [--clang PATH]

PROFILE is the file written by `src32_testbench --profile-pc PATH`: one line per
committed PC, `K|U pc instructions cycles` (K = supervisor mode, U = user mode).
ASM is the assembly of the profiled image, which `tools/llvm-src32/driver.py`
keeps with `--save-temps DIR` (DIR/output.a).
"""
from __future__ import annotations

import argparse
import bisect
import collections
import importlib.util
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Compiler-made labels inside a function; everything else names a function.
_LOCAL = re.compile(
    r"^__llvm_src32_(block|branch_true|branch_false|select_\w+?|return|switch_\w+?|"
    r"minmax_\w+?|memory_\w+?|unreachable)_\d+$")
_HEX_NAME = re.compile(r"llvm_((?:[0-9a-f]{2})+)")


def load_assembler():
    spec = importlib.util.spec_from_file_location("src32_asm", ROOT / "tools/asm/asm.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def readable(label: str) -> str:
    """`llvm_74616b65` -> `take`; the driver hex-encodes C identifiers."""
    match = _HEX_NAME.fullmatch(label)
    if match:
        try:
            return bytes.fromhex(match.group(1)).decode("ascii")
        except (UnicodeDecodeError, ValueError):
            pass
    return label


def function_symbols(asm_text: str, base_dir: str = ".") -> list[tuple[int, str]]:
    """Sorted (address, function name) pairs, without block labels and data."""
    assembler = load_assembler().Assembler(strip=True, sparse=True)
    assembler.assemble(asm_text, source="<pcprof>", base_dir=base_dir)
    return sorted((address, readable(label)) for label, address in assembler.symbols.items()
                  if not _LOCAL.match(label) and not readable(label).startswith("."))


def read_profile(text: str):
    """Yields (user, pc, instructions, cycles)."""
    for line in text.splitlines():
        if not line.strip():
            continue
        mode, pc, count, cycles = line.split()
        yield mode == "U", int(pc, 16), int(count), int(cycles)


class Symbols:
    def __init__(self, symbols: list[tuple[int, str]]):
        self.symbols = symbols
        self.addresses = [address for address, _ in symbols]
        self.entry = {name: address for address, name in symbols}

    def name(self, pc: int) -> str:
        index = bisect.bisect_right(self.addresses, pc) - 1
        return self.symbols[index][1] if index >= 0 else "?"


def summarize(profile: str, symbols: Symbols, top: int = 25) -> dict:
    """Aggregates a profile; kernel PCs by function, user PCs by 64 KiB region."""
    funcs = collections.defaultdict(lambda: [0, 0])
    regions = collections.defaultdict(lambda: [0, 0])
    totals = {"kernel": [0, 0], "user": [0, 0]}
    entry_hits = collections.Counter()
    for user, pc, count, cycles in read_profile(profile):
        side = totals["user" if user else "kernel"]
        side[0] += count
        side[1] += cycles
        if user:
            regions[pc >> 16][0] += count
            regions[pc >> 16][1] += cycles
        else:
            name = symbols.name(pc)
            funcs[name][0] += count
            funcs[name][1] += cycles
            if symbols.entry.get(name) == pc:
                entry_hits[name] += count
    kernel_total = totals["kernel"][0] or 1
    rows = []
    for name, (count, cycles) in sorted(funcs.items(), key=lambda item: -item[1][0])[:top]:
        calls = entry_hits[name]
        rows.append({"function": name, "instructions": count, "share": count / kernel_total,
                     "cycles": cycles, "calls": calls,
                     "instructions_per_call": count / calls if calls else None})
    return {"totals": totals, "functions": rows,
            "user_regions": [{"base": base << 16, "instructions": v[0], "cycles": v[1]}
                             for base, v in sorted(regions.items(), key=lambda item: -item[1][0])[:8]]}


def print_summary(summary: dict) -> None:
    kernel, user = summary["totals"]["kernel"], summary["totals"]["user"]
    total = (kernel[0] + user[0]) or 1
    print(f"instructions: kernel {kernel[0]} ({kernel[0] / total:.1%}), user {user[0]} ({user[0] / total:.1%})")
    print(f"{'function':32s} {'instr':>10s} {'share':>6s} {'calls':>7s} {'instr/call':>10s}")
    for row in summary["functions"]:
        per_call = f"{row['instructions_per_call']:10.1f}" if row["instructions_per_call"] else f"{'-':>10s}"
        print(f"{row['function']:32s} {row['instructions']:10d} {row['share']:6.1%} {row['calls']:7d} {per_call}")
    if summary["user_regions"]:
        print("user code by 64 KiB region:")
        for row in summary["user_regions"]:
            print(f"  0x{row['base']:08X} {row['instructions']:10d}")
    print("Notes: 'calls' counts executions of the entry instruction; helpers the compiler inlined "
          "are charged to their caller; 'cycles' in --json include the HALT time before a commit.")


def cmd_report(args) -> int:
    asm_path = Path(args.asm)
    if asm_path.is_dir():
        asm_path = asm_path / "output.a"
    symbols = Symbols(function_symbols(asm_path.read_text(encoding="utf-8"), str(asm_path.parent)))
    summary = summarize(Path(args.profile).read_text(), symbols, args.top)
    if args.json:
        print(json.dumps(summary, indent=2))
    else:
        print_summary(summary)
    return 0


def cmd_symbols(args) -> int:
    asm_path = Path(args.asm)
    if asm_path.is_dir():
        asm_path = asm_path / "output.a"
    for address, name in function_symbols(asm_path.read_text(encoding="utf-8"), str(asm_path.parent)):
        if not args.grep or args.grep in name:
            print(f"{address:08X} {name}")
    return 0


def cmd_foundation(args) -> int:
    """Runs a shell command on the Foundation image and profiles the kernel."""
    import os
    import shutil
    import subprocess

    out = Path(args.outdir).resolve()
    out.mkdir(parents=True, exist_ok=True)
    if args.clang:
        os.environ["CLANG"] = args.clang
    sys.path[:0] = [str(ROOT / "foundation/tests/shell"), str(ROOT / "foundation/boot"),
                    str(ROOT / "tools/wax")]
    import build as boot_build
    import test_shell
    import wax

    profile = out / "pc_profile.txt"
    shell = test_shell.ShellTests
    original_run = subprocess.run

    def run_with_profile(command, *a, **k):
        if isinstance(command, list) and command and str(command[0]) == str(shell.harness):
            command = command + ["--profile-pc", str(profile), "--profile-pc-reset-frame", str(args.reset_frame)]
        return original_run(command, *a, **k)

    test_shell.subprocess.run = run_with_profile
    test_shell.FRAMES = args.frames
    shell.setUpClass()
    try:
        typed = args.command.replace("\\", "\\\\") + "\\n"      # the testbench unescapes \n itself
        rows = shell().screen(typed, at_frames=[30], cpu="pipeline")
        print("\n".join(rows[:10]))
        built = wax.parse((shell.work / "foundation.wax").read_bytes())
    finally:
        shell.tearDownClass()

    # The kernel is rebuilt with --save-temps to keep its assembly for the symbols.
    temps = out / "temps"
    shutil.rmtree(temps, ignore_errors=True)
    kernel_args = ["--origin", hex(boot_build.KERNEL_LOAD), "--stack-top", hex(boot_build.KERNEL_STACK_TOP),
                   "-D", f"FOUNDATION_BOOT_STOP_TICKS={test_shell.STOP_TICKS}u"]
    rebuilt = out / "foundation.wax"
    boot_build.driver([boot_build.HERE / "main.c", *boot_build.KERNEL], rebuilt,
                      argparse.Namespace(clang=os.environ.get("CLANG"), optimization="-O1"),
                      "-D", "FOUNDATION_BOOT_EMBEDDED", *kernel_args, "--save-temps", str(temps))
    def kernel_block(image):
        return next(b["data"] for b in image["blocks"] if b["load_address"] == boot_build.KERNEL_LOAD)
    if kernel_block(wax.parse(rebuilt.read_bytes())) != kernel_block(built):
        print("warning: the rebuilt kernel differs from the profiled one; the symbols may be off",
              file=sys.stderr)
    return cmd_report(argparse.Namespace(profile=str(profile), asm=str(temps), top=args.top, json=False))


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command_name", required=True)

    report = sub.add_parser("report", help="symbolize a --profile-pc file")
    report.add_argument("profile")
    report.add_argument("asm", help="output.a, or the --save-temps directory holding it")
    report.add_argument("--top", type=int, default=25)
    report.add_argument("--json", action="store_true")
    report.set_defaults(handler=cmd_report)

    symbols = sub.add_parser("symbols", help="list function addresses of an assembly file")
    symbols.add_argument("asm")
    symbols.add_argument("--grep")
    symbols.set_defaults(handler=cmd_symbols)

    foundation = sub.add_parser("foundation", help="profile the Foundation kernel under a shell command")
    foundation.add_argument("outdir")
    foundation.add_argument("--command", default="top -n 4", help="typed at the shell prompt (default: top -n 4)")
    foundation.add_argument("--reset-frame", type=int, default=140,
                            help="clear the histogram at this frame, after boot (default 140)")
    foundation.add_argument("--frames", type=int, default=700, help="frames to run (default 700)")
    foundation.add_argument("--top", type=int, default=25)
    foundation.add_argument("--clang", help="Clang matching llvmlite's LLVM major version")
    foundation.set_defaults(handler=cmd_foundation)

    args = parser.parse_args(argv)
    return args.handler(args)


if __name__ == "__main__":
    sys.exit(main())
