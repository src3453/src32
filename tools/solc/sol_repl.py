"""Interactive REPL for sol VM."""

from __future__ import annotations

from typing import Callable

from sol_vm import SolVM, SolVMError


def run_repl(
    vm: SolVM | None = None,
    input_fn: Callable[[str], str] = input,
    output_fn: Callable[[str], None] = print,
) -> int:
    runtime = vm or SolVM()
    output_fn("sol repl (type .help)")

    while True:
        try:
            line = input_fn("sol> ")
        except EOFError:
            output_fn("")
            return 0

        src = line.strip()
        if src == "":
            continue

        if src in {".quit", ".exit"}:
            return 0
        if src == ".help":
            output_fn(".stack  show current stack")
            output_fn(".trace  show or toggle instruction trace")
            output_fn(".reset  reset VM state")
            output_fn(".quit   exit repl")
            continue
        if src == ".stack":
            output_fn(f"stack: {runtime.stack}")
            continue
        if src.startswith(".trace"):
            parts = src.split()
            if len(parts) == 1:
                status = "on" if runtime.trace_enabled else "off"
                output_fn(f"trace: {status}")
                if runtime.trace:
                    output_fn("trace entries:")
                    for entry in runtime.trace:
                        output_fn(f"  {entry}")
            elif len(parts) == 2 and parts[1] in {"on", "off"}:
                runtime.set_trace(parts[1] == "on")
                output_fn(f"trace {'enabled' if runtime.trace_enabled else 'disabled'}")
            else:
                output_fn("usage: .trace [on|off]")
            continue
        if src == ".reset":
            runtime.reset()
            output_fn("vm reset")
            continue

        try:
            runtime.run_source(src)
            if runtime.trace_enabled and runtime.trace:
                output_fn("trace:")
                for entry in runtime.trace:
                    output_fn(f"  {entry}")
        except SolVMError as exc:
            output_fn(f"error: {exc}")


def main() -> int:
    return run_repl()


if __name__ == "__main__":
    raise SystemExit(main())
