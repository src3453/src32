"""solc CLI (phase 1 scaffold)."""

from __future__ import annotations

import argparse
import logging
import sys

from sol_compiler import SolCompileError, compile_to_src32_asm
from sol_repl import run_repl
from sol_vm import SolVM, SolVMError


def configure_logging() -> None:
    logging.basicConfig(level=logging.ERROR, format="\033[91m%(levelname)s: %(message)s\033[0m")
    logging.basicConfig(level=logging.WARNING, format="\033[93m%(levelname)s: %(message)s\033[0m")
    logging.basicConfig(level=logging.INFO, format="\033[37m%(message)s\033[0m")
    logging.basicConfig(level=logging.DEBUG, format="\033[90m%(message)s\033[0m")


def run_file(input_path: str, trace: bool = False) -> int:
    with open(input_path, "r", encoding="utf-8") as f:
        src = f.read()
    vm = SolVM()
    vm.set_trace(trace)
    try:
        stack = vm.run_source(src, source_path=input_path)
    except SolVMError as exc:
        print(f"\033[91msol runtime error: {exc}\033[0m", file=sys.stderr)
        return 1
    if trace and vm.trace:
        print("trace:")
        for entry in vm.trace:
            print(f"  {entry}")
    print(f"stack: {stack}")
    return 0


def _parse_int_option(s: str) -> int:
    # accept hex (0x...) or decimal
    try:
        return int(s, 0)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"invalid integer value: {s}") from exc


def compile_stub(input_path: str, out_path: str | None, debug: bool=False, var_base: int = 0x00100000, stack_top: int = 0x0000FFFC, read_only_data_base: int = 0x00020000, use_short_mode: bool = True, remove_unused_functions: bool = True) -> int:
    with open(input_path, "r", encoding="utf-8") as f:
        src = f.read()
    try:
        asm = compile_to_src32_asm(
            src,
            debug=debug,
            var_base=var_base,
            stack_top=stack_top,
            read_only_data_base=read_only_data_base,
            source_path=input_path,
            use_short_mode=use_short_mode,
            remove_unused_functions=remove_unused_functions,
        )
    except SolCompileError as exc:
        print(f"\033[91msol compile error: {exc}\033[0m", file=sys.stderr)
        return 1
    if out_path:
        with open(out_path, "w", encoding="utf-8") as f:
            f.write(asm)
        print(f"\033[92mWrote output to {out_path}\033[0m")
    else:
        print(asm)
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="sol tools (VM/REPL and compiler scaffold)")
    subparsers = parser.add_subparsers(dest="command", required=True)

    run_parser = subparsers.add_parser("run", help="run sol source on Python VM")
    run_parser.add_argument("input", help="sol source file")
    run_parser.add_argument("--trace", action="store_true", help="print step-by-step VM trace output")

    subparsers.add_parser("repl", help="start sol REPL")

    compile_parser = subparsers.add_parser("compile", help="compile sol to SRC32 assembly")
    compile_parser.add_argument("input", help="sol source file")
    compile_parser.add_argument("-o", "--out", help="output assembly path")
    compile_parser.add_argument("--debug", action="store_true", help="include debug comments in output")
    compile_parser.add_argument("--no-short-mode", action="store_true", help="disable automatic Short Mode instruction selection")
    compile_parser.add_argument("--keep-unused-functions", action="store_true", help="keep function definitions that are not reachable from the top-level program")
    compile_parser.add_argument("--var-base", type=_parse_int_option, default=0x00100000, help="base address to allocate global variables (default: 0x00100000)")
    compile_parser.add_argument("--stack-top", type=_parse_int_option, default=0x000FFFFC, help="initial stack top address for R28 (default: 0x000FFFFC)")
    compile_parser.add_argument("--read-only-data-base", type=_parse_int_option, default=0x00020000, help="base address for read-only data (default: 0x20000)")
    return parser


def main(argv: list[str] | None = None) -> int:
    configure_logging()
    parser = build_parser()
    args = parser.parse_args(argv)

    if args.command == "run":
        return run_file(args.input, trace=args.trace)
    if args.command == "repl":
        return run_repl()
    if args.command == "compile":
        return compile_stub(
            args.input,
            args.out,
            debug=args.debug,
            var_base=args.var_base,
            stack_top=args.stack_top,
            read_only_data_base=args.read_only_data_base,
            use_short_mode=not args.no_short_mode,
            remove_unused_functions=not args.keep_unused_functions,
        )
    parser.error(f"unsupported command: {args.command}")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())