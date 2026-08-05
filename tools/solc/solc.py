"""solc CLI (phase 1 scaffold)."""

from __future__ import annotations

import argparse
import sys

from sol_compiler import SolCompileError, compile_to_src32_asm
from sol_repl import run_repl
from sol_vm import SolVM, SolVMError


def run_file(input_path: str) -> int:
    src = open(input_path, "r", encoding="utf-8").read()
    vm = SolVM()
    try:
        stack = vm.run_source(src)
    except SolVMError as exc:
        print(f"sol runtime error: {exc}", file=sys.stderr)
        return 1
    print(f"stack: {stack}")
    return 0


def _parse_int_option(s: str) -> int:
    # accept hex (0x...) or decimal
    try:
        return int(s, 0)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"invalid integer value: {s}") from exc


def compile_stub(input_path: str, out_path: str | None, debug: bool=False, var_base: int = 0x00100000, stack_top: int = 0x0000FFFC) -> int:
    src = open(input_path, "r", encoding="utf-8").read()
    try:
        asm = compile_to_src32_asm(src, debug=debug, var_base=var_base, stack_top=stack_top)
    except SolCompileError as exc:
        print(f"sol compile error: {exc}", file=sys.stderr)
        return 1
    if out_path:
        with open(out_path, "w", encoding="utf-8") as f:
            f.write(asm)
        print(f"Wrote output to {out_path}")
    else:
        print(asm)
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="sol tools (VM/REPL and compiler scaffold)")
    subparsers = parser.add_subparsers(dest="command", required=True)

    run_parser = subparsers.add_parser("run", help="run sol source on Python VM")
    run_parser.add_argument("input", help="sol source file")

    subparsers.add_parser("repl", help="start sol REPL")

    compile_parser = subparsers.add_parser("compile", help="compile sol to SRC32 assembly")
    compile_parser.add_argument("input", help="sol source file")
    compile_parser.add_argument("-o", "--out", help="output assembly path")
    compile_parser.add_argument("--debug", action="store_true", help="include debug comments in output")
    compile_parser.add_argument("--var-base", type=_parse_int_option, default=0x00100000, help="base address to allocate global variables (default: 0x00100000)")
    compile_parser.add_argument("--stack-top", type=_parse_int_option, default=0x000FFFFC, help="initial stack top address for R28 (default: 0x000FFFFC)")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    if args.command == "run":
        return run_file(args.input)
    if args.command == "repl":
        return run_repl()
    if args.command == "compile":
        return compile_stub(args.input, args.out, debug=args.debug, var_base=args.var_base, stack_top=args.stack_top)
    parser.error(f"unsupported command: {args.command}")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())