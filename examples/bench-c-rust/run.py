"""Builds the same benchmark kernels in C and in Rust with llvm-src32, runs both
on src32_testbench and compares speed (CPU cycles), peak stack and code size.

Run from anywhere:

    python examples/bench-c-rust/run.py [--c-opt 2] [--rust-opt 2] [--cpu pipeline] [--json OUT]

See README.md for what is measured. Needs Clang, Cargo with rust-src (rustc's
LLVM major version equal to llvmlite's) and src32_testbench (SRC32_TESTBENCH,
CLANG and CARGO override discovery as in the toolchain tests).
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import zlib

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
LLVM_SRC32 = ROOT / "tools" / "llvm-src32"
SDK = ROOT / "sdk" / "cpt32"

KERNELS = ("crc32", "sieve", "sort", "matmul", "nqueens", "mandel")
CPUS = ("pipeline", "cached", "cached-relaxed", "scalar", "jit")
RUST_OPT_LEVELS = ("0", "1", "2", "3", "s", "z")


class BenchError(Exception):
    pass


# ---------------------------------------------------------------- reference


def lcg(seed: int):
    state = seed
    while True:
        state = (state * 1664525 + 1013904223) & 0xFFFFFFFF
        yield state


def reference() -> dict[str, int]:
    """The checksums every kernel must return (harness inputs, README definitions)."""
    values = {}
    numbers = lcg(1)
    values["crc32"] = zlib.crc32(bytes(next(numbers) >> 24 for _ in range(16384)))

    n = 32768
    flags = bytearray([1]) * n
    flags[0] = flags[1] = 0
    i = 2
    while i * i < n:
        if flags[i]:
            flags[i * i::i] = bytes(len(range(i * i, n, i)))
        i += 1
    values["sieve"] = sum(flags)

    numbers = lcg(2)
    data = sorted(next(numbers) for _ in range(2000))
    values["sort"] = sum(value * (index + 1) for index, value in enumerate(data)) & 0xFFFFFFFF

    n = 24
    numbers = lcg(3)
    raw = [(next(numbers) >> 28) - 8 for _ in range(2 * n * n)]
    a, b = raw[:n * n], raw[n * n:]
    digest = 0
    for i in range(n):
        for j in range(n):
            value = sum(a[i * n + k] * b[k * n + j] for k in range(n)) & 0xFFFFFFFF
            digest = (digest * 31 + value) & 0xFFFFFFFF
    values["matmul"] = digest

    def solve(columns, left, right, full):
        if columns == full:
            return 1
        count = 0
        free = full & ~(columns | left | right)
        while free:
            bit = free & -free
            free -= bit
            count += solve(columns | bit, ((left | bit) << 1) & 0xFFFFFFFF, (right | bit) >> 1, full)
        return count

    values["nqueens"] = solve(0, 0, 0, (1 << 9) - 1)

    one = 4096
    width = 48
    height = width * 2 // 3
    total = 0
    for y in range(height):
        ci = -one + 2 * one * y // height
        for x in range(width):
            cr = -10 * one // 4 + 14 * one // 4 * x // width
            zr = zi = 0
            iteration = 0
            while iteration < 32:
                zr2 = (zr * zr) >> 12
                zi2 = (zi * zi) >> 12
                if zr2 + zi2 > 4 * one:
                    break
                zi = ((zr * zi) >> 11) + ci
                zr = zr2 - zi2 + cr
                iteration += 1
            total += iteration
    values["mandel"] = total
    return values


# ---------------------------------------------------------------- building


def find_testbench(explicit: str | None) -> Path:
    if explicit:
        return Path(explicit)
    if os.environ.get("SRC32_TESTBENCH"):
        return Path(os.environ["SRC32_TESTBENCH"])
    suffix = ".exe" if os.name == "nt" else ""
    for profile in ("release", "debug"):
        candidate = ROOT / "target" / profile / ("src32_testbench" + suffix)
        if candidate.is_file():
            return candidate
    raise BenchError("src32_testbench not found: cargo build --release --bin src32_testbench, "
                     "or pass --testbench / set SRC32_TESTBENCH")


def harness_inputs(c_opt: str) -> list[str]:
    return [str(HERE / "harness" / "harness.c"), str(SDK / "src" / "print.c"),
            "-I", str(SDK / "include"), "-I", str(HERE / "harness"), f"-O{c_opt}"]


def run_tool(command: list[str], env=None, verbose=False) -> None:
    if verbose:
        print(" ".join(command), file=sys.stderr)
    result = subprocess.run(command, env=env, capture_output=not verbose, text=True,
                            encoding="utf-8", errors="replace")
    if result.returncode:
        output = "" if verbose else result.stdout + result.stderr
        raise BenchError(f"command failed ({result.returncode}): {' '.join(command)}\n{output}")


def build_c(work: Path, c_opt: str, clang: str | None, verbose: bool) -> tuple[Path, Path]:
    temps = work / "c-temps"
    image = work / "bench-c.wax"
    command = [sys.executable, str(LLVM_SRC32 / "driver.py"), str(HERE / "c" / "kernels.c"),
               *harness_inputs(c_opt), "--save-temps", str(temps), "-o", str(image)]
    if clang:
        command += ["--clang", clang]
    run_tool(command, verbose=verbose)
    return image, temps / "output.a"


def build_rust(work: Path, c_opt: str, rust_opt: str, clang: str | None, verbose: bool) -> tuple[Path, Path]:
    temps = work / "rust-temps"
    image = work / "bench-rust.wax"
    command = [sys.executable, str(LLVM_SRC32 / "cargo_src32.py"), "--manifest-path", str(HERE / "rust" / "Cargo.toml"),
               "--target-dir", str(work / "cargo"), *harness_inputs(c_opt),
               "--save-temps", str(temps), "-o", str(image)]
    if clang:
        command += ["--clang", clang]
    env = dict(os.environ)
    env["CARGO_PROFILE_RELEASE_OPT_LEVEL"] = rust_opt
    run_tool(command, env=env, verbose=verbose)
    return image, temps / "output.a"


# ---------------------------------------------------------------- code size


def load_assembler():
    spec = importlib.util.spec_from_file_location("_bench_src32_asm", ROOT / "tools" / "asm" / "asm.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


LABEL = re.compile(r"^([A-Za-z_.$][\w.$]*):\s*$")


def symbol_name(label: str) -> str:
    """llvm-src32 writes an IR name `x` as `llvm_<hex of x>`."""
    if label.startswith("llvm_"):
        try:
            return bytes.fromhex(label[5:]).decode("utf-8", "replace")
        except ValueError:
            pass
    return label


def symbol_sizes(assembly_path: Path) -> dict:
    """Code and data bytes of every top-level label of the assembly."""
    text = assembly_path.read_text(encoding="utf-8")
    lines = text.splitlines()
    labels = []  # (label, is_data)
    for index, line in enumerate(lines):
        match = LABEL.match(line)
        if not match:
            continue
        label = match.group(1)
        # Basic blocks, branch targets and the like are local to a function.
        if label.startswith("__llvm_src32_") and label != "__llvm_src32_start":
            continue
        following = next((other.strip() for other in lines[index + 1:]
                          if other.strip() and not LABEL.match(other)), "")
        labels.append((label, following.startswith((".BYTE", ".WORD", ".HALF", ".ORG", ".ASCII"))))
    assembler = load_assembler().Assembler(strip=True, sparse=True)
    assembler.assemble(text, source=str(assembly_path), base_dir=str(ROOT))
    end = max(address + len(data) for address, data in assembler.blocks)
    placed = sorted((assembler.symbols[label], label, is_data) for label, is_data in labels)
    symbols = []
    for position, (address, label, is_data) in enumerate(placed):
        following = placed[position + 1][0] if position + 1 < len(placed) else end
        symbols.append({"name": symbol_name(label), "address": address, "size": following - address,
                        "data": is_data})
    return {"end": end, "symbols": symbols}


def kernel_of(name: str) -> str | None:
    """bench_<kernel> (C functions) or a bench_<kernel> path segment (Rust v0 mangling)."""
    for kernel in KERNELS:
        if re.search(rf"bench_{kernel}(?![a-z])", name):
            return kernel
    return None


def size_summary(layout: dict) -> dict:
    code = sum(s["size"] for s in layout["symbols"] if not s["data"])
    data = sum(s["size"] for s in layout["symbols"] if s["data"])
    kernels = {kernel: 0 for kernel in KERNELS}
    for symbol in layout["symbols"]:
        kernel = kernel_of(symbol["name"])
        if kernel and not symbol["data"]:
            kernels[kernel] += symbol["size"]
    return {"image": layout["end"], "code": code, "data": data, "kernels": kernels,
            "other_code": code - sum(kernels.values())}


# ---------------------------------------------------------------- running


def run_image(testbench: Path, image: Path, cpu: str, frames: int) -> dict:
    with tempfile.TemporaryDirectory(prefix="bench-c-rust-") as directory:  # no BIOS config.yaml
        command = [str(testbench), str(image), "0", "--cpu", cpu, "--frames", str(frames),
                   "--dump-uart", "uart.txt"]
        result = subprocess.run(command, cwd=directory, capture_output=True, text=True,
                                encoding="utf-8", errors="replace", timeout=1800)
        if result.returncode:
            raise BenchError(f"{image.name} failed on src32_testbench:\n{result.stdout}{result.stderr}")
        uart = (Path(directory) / "uart.txt").read_bytes().decode("ascii", "replace")
    report = {"results": {}}
    for line in uart.splitlines():
        fields = line.split()
        if not fields:
            continue
        if fields[0] == "INFO":
            report[fields[1]] = [int(field) for field in fields[2:]]
        elif fields[0] == "RESULT":
            cycles, stack, value = (int(field) for field in fields[2:5])
            report["results"][fields[1]] = {"cycles": cycles, "stack": stack, "value": value}
        elif fields[0] == "DONE":
            report["done"] = True
    if not report.get("done"):
        raise BenchError(f"{image.name} did not finish:\n{uart}")
    return report


def check(report: dict, layout: dict, expected: dict[str, int], language: str) -> list[str]:
    problems = []
    for kernel in KERNELS:
        got = report["results"][kernel]
        if got["value"] != expected[kernel]:
            problems.append(f"{language} {kernel}: checksum {got['value']:#x}, expected {expected[kernel]:#x}")
        if got["stack"] == 0xFFFFFFFF:
            problems.append(f"{language} {kernel}: the stack went below the painted area")
    if report["paint"][0] < layout["end"]:
        problems.append(f"{language}: the painted stack area overlaps the image (ends at {layout['end']:#x})")
    return problems


def summarize(report: dict) -> dict:
    overhead = report["overhead"][0]
    baseline = report["results"]["empty"]["stack"]
    return {kernel: {"cycles": report["results"][kernel]["cycles"] - overhead,
                     "stack": report["results"][kernel]["stack"] - baseline,
                     "value": report["results"][kernel]["value"]} for kernel in KERNELS}


def ratio(rust: int, c: int) -> str:
    return f"{rust / c:.2f}" if c else "-"


def table(results: dict, sizes: dict) -> str:
    rows = ["| kernel | C cycles | Rust cycles | Rust/C | C stack | Rust stack | C code | Rust code |",
            "|---|--:|--:|--:|--:|--:|--:|--:|"]
    for kernel in KERNELS:
        c, rust = results["c"][kernel], results["rust"][kernel]
        c_code, rust_code = sizes["c"]["kernels"][kernel], sizes["rust"]["kernels"][kernel]
        rows.append(f"| {kernel} | {c['cycles']:,} | {rust['cycles']:,} | {ratio(rust['cycles'], c['cycles'])} "
                    f"| {c['stack']:,} | {rust['stack']:,} | {c_code:,} | {rust_code:,} |")
    c_total = sum(results["c"][kernel]["cycles"] for kernel in KERNELS)
    rust_total = sum(results["rust"][kernel]["cycles"] for kernel in KERNELS)
    rows.append(f"| **total** | {c_total:,} | {rust_total:,} | {ratio(rust_total, c_total)} | | "
                f"| {sum(sizes['c']['kernels'].values()):,} | {sum(sizes['rust']['kernels'].values()):,} |")
    rows += ["", "| image (bytes) | C | Rust | Rust/C |", "|---|--:|--:|--:|"]
    for key, title in (("code", "code"), ("other_code", "  harness + runtime + core"),
                       ("data", "data (incl. zero-initialised)"), ("image", "image end")):
        rows.append(f"| {title} | {sizes['c'][key]:,} | {sizes['rust'][key]:,} "
                    f"| {ratio(sizes['rust'][key], sizes['c'][key])} |")
    return "\n".join(rows)


def largest_other(layout: dict, count: int) -> list[tuple[int, str]]:
    others = [(s["size"], s["name"]) for s in layout["symbols"] if not s["data"] and not kernel_of(s["name"])]
    return sorted(others, reverse=True)[:count]


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--c-opt", choices=("0", "1", "2", "s", "z"), default="2",
                   help="Clang -O level of the C kernels and of the shared harness (default 2)")
    p.add_argument("--rust-opt", choices=RUST_OPT_LEVELS, default="2", help="Rust opt-level (default 2)")
    p.add_argument("--cpu", choices=CPUS, default="pipeline", help="src32_testbench CPU model (default pipeline)")
    p.add_argument("--frames", type=int, default=400, help="testbench frame budget, 800,000 cycles each")
    p.add_argument("--work", type=Path, default=ROOT / "target" / "bench-c-rust",
                   help="build directory (default target/bench-c-rust)")
    p.add_argument("--testbench", help="src32_testbench executable")
    p.add_argument("--clang", default=os.environ.get("CLANG"), help="Clang executable")
    p.add_argument("--json", type=Path, help="also write the measurements as JSON")
    p.add_argument("--symbols", type=int, default=0, metavar="N",
                   help="also list the N largest non-kernel functions of each image")
    p.add_argument("-v", "--verbose", action="store_true", help="show the build commands and their output")
    return p


def main(argv=None) -> int:
    args = parser().parse_args(argv)
    try:
        testbench = find_testbench(args.testbench)
        args.work.mkdir(parents=True, exist_ok=True)
        expected = reference()
        builds = {"c": build_c(args.work, args.c_opt, args.clang, args.verbose),
                  "rust": build_rust(args.work, args.c_opt, args.rust_opt, args.clang, args.verbose)}
        layouts, sizes, reports, results, problems = {}, {}, {}, {}, []
        for language, (image, assembly) in builds.items():
            layouts[language] = symbol_sizes(assembly)
            sizes[language] = size_summary(layouts[language])
            reports[language] = run_image(testbench, image, args.cpu, args.frames)
            problems += check(reports[language], layouts[language], expected, language)
            results[language] = summarize(reports[language])
    except BenchError as exc:
        print(f"bench-c-rust: error: {exc}", file=sys.stderr)
        return 1
    print(f"C -O{args.c_opt}, Rust opt-level {args.rust_opt}, --cpu {args.cpu} "
          f"(cycles of the 48 MHz system clock; stack and code in bytes)\n")
    print(table(results, sizes))
    for language in ("c", "rust") if args.symbols else ():
        print(f"\nlargest non-kernel functions ({language}):")
        for size, name in largest_other(layouts[language], args.symbols):
            print(f"  {size:8,}  {name}")
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps({
            "options": {"c_opt": args.c_opt, "rust_opt": args.rust_opt, "cpu": args.cpu},
            "expected": expected, "results": results, "sizes": sizes,
            "raw": reports, "problems": problems}, indent=2), encoding="utf-8")
    for problem in problems:
        print(f"bench-c-rust: error: {problem}", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    raise SystemExit(main())
