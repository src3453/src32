"""solc CLI (phase 1 scaffold)."""

from __future__ import annotations

import argparse
import sys

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


def compile_stub(input_path: str, out_path: str | None) -> int:
    _ = open(input_path, "r", encoding="utf-8").read()
    _lexer_stub = "phase1-lexer-placeholder"
    _parser_stub = "phase1-parser-placeholder"
    _ir_stub = "phase1-ir-placeholder"
    _ = (_lexer_stub, _parser_stub, _ir_stub, out_path)
    print("compile is not implemented yet (phase 1: VM/REPL first)", file=sys.stderr)
    return 2


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="sol tools (VM/REPL and compiler scaffold)")
    subparsers = parser.add_subparsers(dest="command", required=True)

    run_parser = subparsers.add_parser("run", help="run sol source on Python VM")
    run_parser.add_argument("input", help="sol source file")

    subparsers.add_parser("repl", help="start sol REPL")

    compile_parser = subparsers.add_parser("compile", help="compile sol to SRC32 (stub)")
    compile_parser.add_argument("input", help="sol source file")
    compile_parser.add_argument("-o", "--out", help="output assembly path")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    if args.command == "run":
        return run_file(args.input)
    if args.command == "repl":
        return run_repl()
    if args.command == "compile":
        return compile_stub(args.input, args.out)
    parser.error(f"unsupported command: {args.command}")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())