#!/usr/bin/env python3
"""Freestanding big-endian C / C++ / LLVM IR to SRC32 assembler and WAX."""
from __future__ import annotations

import argparse
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent
RUNTIME_NAMES = ("memcpy", "memmove", "memset", "memcmp", "strlen")
CXX_SUFFIXES = (".cpp", ".cc", ".cxx")
SOURCE_SUFFIXES = (".c", *CXX_SUFFIXES)


class DriverError(Exception):
    pass


def address(text: str) -> int:
    try:
        value = int(text, 0)
    except ValueError:
        raise argparse.ArgumentTypeError("expected an integer address (decimal or 0x hexadecimal)") from None
    if not 0 <= value <= 0xFFFFFFFF:
        raise argparse.ArgumentTypeError("address must fit in 32 bits")
    return value


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    p.add_argument("inputs", nargs="+", type=Path, help="C/C++, textual LLVM IR (.ll), or LLVM bitcode (.bc)")
    p.add_argument("-o", "--output", type=Path)
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("-S", action="store_true", help="emit SRC32 assembly")
    mode.add_argument("--emit-llvm", action="store_true", help="emit linked, verified LLVM IR")
    p.add_argument("-I", dest="includes", action="append", default=[])
    p.add_argument("-D", dest="defines", action="append", default=[])
    opt = p.add_mutually_exclusive_group()
    for level in range(3):
        opt.add_argument(f"-O{level}", dest="optimization", action="store_const", const=f"-O{level}")
    p.set_defaults(optimization="-O0")
    p.add_argument("--origin", type=address, default=0)
    p.add_argument("--stack-top", type=address, default=0x00100000)
    p.add_argument("--data-origin", type=address,
                   help="ROM layout: place writable globals in RAM at this address; startup copies "
                        "their initial values from the image and clears zero-initialized data")
    p.add_argument("--entry", default="main", help="C/LLVM entry function (default: main)")
    p.add_argument("--protected", action="store_true",
                   help="emit an Extension P user-mode program as a PROTECTED .wax "
                        "(code R/X, data R/W, user stack below --stack-top)")
    p.add_argument("--user-stack-size", type=address, default=0x4000,
                   help="user stack bytes below --stack-top with --protected (default 0x4000)")
    p.add_argument("--save-temps", type=Path, metavar="DIR")
    p.add_argument("--depfile", type=Path, metavar="PATH", help="write Make-style dependencies after successful compilation")
    p.add_argument("--clang", help="Clang executable path; otherwise use CLANG or PATH")
    p.add_argument("--newlib-source", type=Path, help="external Newlib source root (optional portable integer/string/allocator profile)")
    p.add_argument("--newlib-heap-size", type=address, default=65536, help="reserved Newlib heap bytes (default 65536; multiple of 8)")
    return p


def find_clang(explicit: str | None) -> str:
    selected = explicit if explicit is not None else os.environ.get("CLANG")
    if selected:
        found = shutil.which(selected)
        if found:
            return found
        raise DriverError(f"Clang executable not found: {selected!r}; supply --clang PATH")
    found = shutil.which("clang")
    if found:
        return found
    # ROCm ships a conventional Clang distribution on Windows.
    candidates = sorted(Path("C:/Program Files/AMD/ROCm").glob("*/bin/clang.exe"), reverse=True)
    if candidates:
        return str(candidates[0])
    raise DriverError("Clang not found; install Clang and add it to PATH, or set --clang PATH / CLANG")


def compile_c(clang: str, source: Path, output: Path, args, defines=(), runtime=False, depfile=None) -> None:
    cxx = source.suffix.lower() in CXX_SUFFIXES
    command = [clang, "--target=mips-unknown-none", "-mabi=o32", "-msoft-float",
               "-ffreestanding", "-fno-builtin", "-fno-stack-protector", "-fno-pic", "-fno-pie",
               "-fno-vectorize", "-fno-slp-vectorize", "-fsigned-char", "-nostdinc",
               "-std=c++17" if cxx else "-std=c11",
               "-O0" if runtime else args.optimization, "-S", "-emit-llvm"]
    newlib = getattr(args, "newlib", None)
    for include in newlib.headers() if newlib else [ROOT / "runtime" / "include"]:
        command.extend(["-isystem", str(include)])
    if newlib:
        command.append("-DPREFER_SIZE_OVER_SPEED=1")
        if runtime and source.name == "nano-mallocr.c":
            command.extend(["-include", str(ROOT / "runtime/newlib/include/port.h")])
    if cxx:
        command.extend(["-fno-exceptions", "-fno-rtti", "-fno-threadsafe-statics",
                        "-fno-use-cxa-atexit", "-Xclang", "-mno-constructor-aliases"])
    # Also set LLVM's pipeline switches: ROCm Clang 20 otherwise runs its SLP
    # pass at -O2 despite the frontend -fno-slp-vectorize flag.
    command.extend(["-mllvm", "-vectorize-slp=false", "-mllvm", "-vectorize-loops=false"])
    if not runtime:
        for include in args.includes:
            command.extend(["-I", include])
        for define in args.defines:
            command.extend(["-D", define])
    command.extend(f"-D{name}" for name in defines)
    if depfile is not None:
        command.extend(["-MD", "-MF", str(depfile.resolve()), "-MT", "src32_dependency_target"])
    command.extend([str(source.resolve()), "-o", str(output.resolve())])
    result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, errors="replace")
    if result.returncode:
        raise DriverError(f"Clang failed for {source}:\n{result.stderr.strip() or result.stdout.strip()}")


def load_python(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise DriverError(f"cannot load tool {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def read_dependencies(path: Path) -> set[Path]:
    """Decode Clang's single Make rule (including escaped filenames)."""
    text = path.read_text(encoding="utf-8").replace("\\\r\n", "").replace("\\\n", "")
    _, separator, text = text.partition(":")
    if not separator:
        raise DriverError(f"invalid Clang dependency file: {path}")
    words, word = [], []
    index = 0
    while index < len(text):
        char = text[index]
        if char == "\\" and index + 1 < len(text) and (text[index + 1].isspace() or text[index + 1] in "\\#:"):
            index += 1
            word.append(text[index])
        elif char == "$" and index + 1 < len(text) and text[index + 1] == "$":
            word.append("$")
            index += 1
        elif char.isspace():
            if word:
                words.append("".join(word))
                word = []
        else:
            word.append(char)
        index += 1
    if word:
        words.append("".join(word))
    return {Path(word).resolve() for word in words}


def make_escape(path: Path) -> str:
    text = path.resolve().as_posix()
    return text.replace("$", "$$").replace("#", "\\#").replace(" ", "\\ ").replace("\t", "\\\t").replace(":", "\\:")


def run(args) -> Path:
    try:
        from llvmlite import binding as llvm
    except ImportError as exc:
        raise DriverError("llvmlite is required; install dependencies with: python -m pip install -r "
                          f'"{ROOT / "requirements.txt"}"') from exc
    from data import Layout

    for source in args.inputs:
        if source.suffix.lower() not in (*SOURCE_SUFFIXES, ".ll", ".bc"):
            raise DriverError(f"unsupported input extension: {source} (expected .c, .cpp, .cc, .cxx, .ll, or .bc)")
        if not source.is_file():
            raise DriverError(f"input file not found: {source}")
    if args.origin % 4:
        raise DriverError("--origin must be 4-byte aligned")
    if not args.stack_top or args.stack_top % 8:
        raise DriverError("--stack-top must be nonzero and 8-byte aligned")
    if args.data_origin is not None and args.data_origin % 8:
        raise DriverError("--data-origin must be 8-byte aligned")
    if args.data_origin is not None and args.protected:
        raise DriverError("--data-origin cannot be combined with --protected")
    if not args.entry or "\x00" in args.entry:
        raise DriverError("--entry must name a function")
    for define in args.defines:
        if not define or define.startswith("-"):
            raise DriverError(f"invalid macro definition: {define!r}")
    if args.protected and (not args.user_stack_size or args.user_stack_size % 8
                           or args.user_stack_size > args.stack_top):
        raise DriverError("--user-stack-size must be a nonzero multiple of 8 that fits below --stack-top")
    if not args.newlib_source and args.newlib_heap_size != 65536:
        raise DriverError("--newlib-heap-size requires --newlib-source")
    output = args.output or args.inputs[0].with_suffix(".ll" if args.emit_llvm else ".a" if args.S else ".wax")
    if args.protected and not (args.emit_llvm or args.S) and output.suffix.lower() != ".wax":
        raise DriverError("--protected needs .wax output: raw .bin cannot carry permissions")
    if not (args.emit_llvm or args.S) and output.suffix.lower() not in (".wax", ".bin"):
        raise DriverError("binary output must have a .wax or .bin extension; use -S for assembly")
    if output.resolve() in {source.resolve() for source in args.inputs}:
        raise DriverError("output must not overwrite an input file")
    if not output.parent.is_dir():
        raise DriverError(f"output directory does not exist: {output.parent}")
    dependencies = {source.resolve() for source in args.inputs}
    if args.depfile:
        if args.depfile.resolve() in dependencies | {output.resolve()}:
            raise DriverError("--depfile must not overwrite an input or output")
        if not args.depfile.parent.is_dir():
            raise DriverError(f"dependency directory does not exist: {args.depfile.parent}")

    def emit_dependencies():
        if args.depfile:
            if args.depfile.resolve() in dependencies:
                raise DriverError("--depfile must not overwrite a source or included header")
            rule = make_escape(output) + ": " + " ".join(make_escape(path) for path in sorted(dependencies)) + "\n"
            args.depfile.write_text(rule, encoding="utf-8")
    clang = None
    if any(source.suffix.lower() in SOURCE_SUFFIXES for source in args.inputs):
        clang = find_clang(args.clang)

    def parse_module(path):
        try:
            module = llvm.parse_bitcode(path.read_bytes()) if path.suffix.lower() == ".bc" else llvm.parse_assembly(path.read_text(encoding="utf-8"))
            module.verify()
            Layout(module)  # Reject incompatible input targets before LLVM can merge them.
            return module
        except (RuntimeError, ValueError) as exc:
            raise DriverError(f"{path}: LLVM parse/verification/layout error:\n{exc}") from exc

    def check_link_layout(reference, candidate, source):
        if str(reference.data_layout) != str(candidate.data_layout):
            raise DriverError(
                f"{source}: LLVM data layout differs from earlier input; "
                "all translation units must use identical SRC32 storage layout")

    with tempfile.TemporaryDirectory(prefix="llvm-src32-") as temporary:
        work = Path(temporary)
        if args.save_temps:
            args.save_temps.mkdir(parents=True, exist_ok=True)
            work = args.save_temps.resolve()
            if any(source.resolve().is_relative_to(work) for source in args.inputs):
                raise DriverError("--save-temps must not contain input files (intermediates may overwrite them)")
        linked = None
        args.newlib = None
        if args.newlib_source:
            from newlib import Newlib
            clang = clang or find_clang(args.clang)
            args.newlib = Newlib(args.newlib_source, clang, args.newlib_heap_size)
            dependencies.add(args.newlib.root / "COPYING.NEWLIB")
        for index, source in enumerate(args.inputs):
            parsed_source = source
            if source.suffix.lower() in SOURCE_SUFFIXES:
                parsed_source = work / f"{index:03d}-{source.stem}.ll"
                dependency_path = parsed_source.with_suffix(".d") if args.depfile else None
                compile_c(clang, source, parsed_source, args, depfile=dependency_path)
                if dependency_path:
                    dependencies.update(read_dependencies(dependency_path))
            try:
                module = parse_module(parsed_source)
            except DriverError as exc:
                raise DriverError(f"input {source}: {exc}") from exc
            if linked is None:
                linked = module
            else:
                check_link_layout(linked, module, source)
                try:
                    linked.link_in(module)
                    linked.verify()
                except RuntimeError as exc:
                    raise DriverError(f"linking {source}: {exc}") from exc
        if args.newlib:
            while True:
                defined = {f.name for f in linked.functions if not f.is_declaration}
                missing = {f.name for f in linked.functions if f.is_declaration} - defined
                pending = args.newlib.pending(missing, defined)
                if not pending:
                    break
                for name, source, defines in pending:
                    ir = work / f"newlib-{name}.ll"
                    dep = ir.with_suffix(".d") if args.depfile else None
                    # Do not leak application defines/includes into upstream libc.
                    compile_c(clang, source, ir, args, defines=defines, runtime=True, depfile=dep)
                    dependencies.add(source.resolve())
                    if dep:
                        dependencies.update(read_dependencies(dep))
                    module = parse_module(ir)
                    check_link_layout(linked, module, source)
                    linked.link_in(module)
                    linked.verify()
        defined = {f.name for f in linked.functions if not f.is_declaration}
        declared = {f.name for f in linked.functions if f.is_declaration}
        needed = set(RUNTIME_NAMES) & declared - defined
        if needed:
            clang = clang or find_clang(args.clang)
            runtime_ir = work / "runtime.ll"
            dependency_path = work / "runtime.d" if args.depfile else None
            compile_c(clang, ROOT / "runtime" / "string.c", runtime_ir, args,
                      defines=[f"SRC32_HAVE_{name}" for name in RUNTIME_NAMES if name not in needed],
                      runtime=True, depfile=dependency_path)
            if dependency_path:
                dependencies.update(read_dependencies(dependency_path))
            try:
                runtime_module = parse_module(runtime_ir)
                check_link_layout(linked, runtime_module, "freestanding runtime")
                linked.link_in(runtime_module)
                linked.verify()
            except RuntimeError as exc:
                raise DriverError(f"linking freestanding runtime: {exc}") from exc
        ir_text = str(linked)
        if args.save_temps:
            (work / "linked.ll").write_text(ir_text, encoding="utf-8")
        if args.emit_llvm:
            output.write_text(ir_text, encoding="utf-8")
            emit_dependencies()
            return output
        from backend import compile_module
        layout_info = {}
        assembly = compile_module(linked, origin=args.origin, stack_top=args.stack_top, entry=args.entry,
                                  protected=args.protected, info=layout_info, data_origin=args.data_origin)
        if args.save_temps:
            (work / "output.a").write_text(assembly, encoding="utf-8")
        if args.S:
            output.write_text(assembly, encoding="utf-8")
        else:
            assembler_module = load_python(ROOT.parent / "asm" / "asm.py", "_llvm_src32_assembler")
            wax = output.suffix.lower() == ".wax"
            assembler = assembler_module.Assembler(strip=True, sparse=wax)
            binary = assembler.assemble(assembly, source="<llvm-src32>", base_dir=str(ROOT))
            if wax:
                wax_module = load_python(ROOT.parent / "wax" / "wax.py", "_llvm_src32_wax")
                blocks = assembler.blocks
                protection = None
                if args.protected:
                    blocks = protected_blocks(blocks, layout_info["data_start"])
                    protection = (args.stack_top, args.user_stack_size)
                try:
                    binary = wax_module.pack(assembler.image_base, args.origin - assembler.image_base,
                                             blocks, protection=protection)
                except ValueError as exc:
                    raise DriverError(f"cannot pack WAX: {exc}") from exc
            output.write_bytes(binary)
        emit_dependencies()
    return output


def protected_blocks(blocks, data_start):
    """Split assembled blocks at the code/data boundary: code R/X, data R/W.

    Alignment padding between the sparse assembler's blocks is filled with
    zeros, so code and data stay one block each however many globals there
    are (PROTECTED WAX allows at most 7 blocks)."""
    perm_rx, perm_rw = 1 | 4, 1 | 2
    result = []
    for address, data in sorted(blocks, key=lambda block: block[0]):
        data = bytes(data)
        split = min(max(data_start - address, 0), len(data))
        parts = []
        if split:
            parts.append((address, data[:split], perm_rx))
        if split < len(data):
            parts.append((address + split, data[split:], perm_rw))
        for part in parts:
            if result and result[-1][2] == part[2] and result[-1][0] + len(result[-1][1]) <= part[0]:
                start, previous, perm = result[-1]
                gap = part[0] - start - len(previous)
                result[-1] = (start, previous + bytes(gap) + part[1], perm)
            else:
                result.append(part)
    return result


def main(argv=None) -> int:
    args = parser().parse_args(argv)
    try:
        output = run(args)
    except Exception as exc:
        # Compiler, verifier and assembler diagnostics are user-facing, not Python failures.
        print(f"llvm-src32: error: {exc}", file=sys.stderr)
        return 1
    print(f"Wrote {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
