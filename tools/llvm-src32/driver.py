#!/usr/bin/env python3
"""Freestanding big-endian C / C++ / LLVM IR to SRC32 assembler and WAX."""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
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
PIE_BASE = 0x00100000  # link address of a relocatable executable; its stack lies just below
PIE_PROBE = 0x01000000  # how far the second link moves it to find the addresses
# OS ABIs a WAX v3 can declare (doc/spec_WAX_v3.md §1): syscall ABI version
# and the SYSCALL number the startup code exits with (spec_Foundation.md §12).
V3_OSES = {"foundation": {"version": (1, 0), "exit": 0x11}}
SOURCE_SUFFIXES = (".c", *CXX_SUFFIXES)
# Berkeley SoftFloat plus the compiler-rt compatible routines (rt.c) that
# floating-point instructions call; see fp.py.
SOFTFLOAT = ROOT / "runtime" / "softfloat"
SOFTFLOAT_INCLUDES = ("port", "source/8086", "source/include")
SOFTFLOAT_DEFINES = ("INLINE_LEVEL=0", "SOFTFLOAT_ROUND_ODD")


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
    p.add_argument("-c", dest="compile_only", action="store_true",
                   help="compile the one C/C++ input to LLVM IR (.ll) with this toolchain's flags and stop: no "
                        "linking, no runtime; for build systems that link the pieces later")
    p.add_argument("-I", dest="includes", action="append", default=[])
    p.add_argument("--include", dest="forced_includes", action="append", default=[], metavar="FILE",
                   help="process FILE before the source, like Clang's -include (application code only)")
    p.add_argument("-D", dest="defines", action="append", default=[])
    opt = p.add_mutually_exclusive_group()
    for level in ("0", "1", "2", "s", "z"):
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
    p.add_argument("--page-align", action="store_true",
                   help="start the data on a 4 KiB boundary after the code, so code (R/X) and data (R/W) "
                        "never share a page; Foundation on Extension V needs this for W^X (use with "
                        "--protected, --wax3 or --wal)")
    p.add_argument("--user-stack-size", type=address, default=0x4000,
                   help="user stack bytes below --stack-top with --protected or --wax3 (default 0x4000)")
    v3 = p.add_argument_group("WAX v3 (doc/spec_WAX_v3.md, call ABI 1)")
    kind = v3.add_mutually_exclusive_group()
    kind.add_argument("--wax3", action="store_true",
                      help="emit a protected WAX v3 executable at --origin (.wax)")
    kind.add_argument("--wal", action="store_true",
                      help="emit a position-independent WAX v3 library (.wal) exporting --exports")
    v3.add_argument("--pie", action="store_true",
                    help="with --wax3, emit a relocatable executable: linked at %#x with the stack just below, "
                         "and a relocation table so the OS can load it anywhere (no-MMU loading)" % PIE_BASE)
    v3.add_argument("--heap-hint", type=address, default=0,
                    help="with --pie, the heap in bytes the program asks the OS to reserve (default: the OS's)")
    v3.add_argument("--image-id", help="image or library ID (default: the output file name)")
    v3.add_argument("--abi-version", default="1.0", metavar="MAJOR.MINOR",
                    help="library ABI version, or the executable's contract version (default 1.0)")
    v3.add_argument("--exports", type=Path, metavar="FILE", help="exports of --wal, one per line")
    v3.add_argument("--import-wal", type=Path, action="append", default=[], metavar="WAL",
                    help="bind undefined symbols to this library's exports (repeatable)")
    v3.add_argument("--os", choices=sorted(V3_OSES), default="foundation",
                    help="OS syscall ABI of the image (default foundation)")
    p.add_argument("--save-temps", type=Path, metavar="DIR")
    p.add_argument("--depfile", type=Path, metavar="PATH", help="write Make-style dependencies after successful compilation")
    p.add_argument("--clang", help="Clang executable path; otherwise use CLANG or PATH")
    p.add_argument("-j", "--jobs", type=int, default=0, metavar="N",
                   help="parallel Clang invocations (default: CPU count; output does not depend on N)")
    p.add_argument("--newlib-source", type=Path, help="external Newlib source root (optional portable integer/string/allocator profile)")
    p.add_argument("--newlib-heap-size", type=address, default=65536, help="reserved Newlib heap bytes (default 65536; multiple of 8)")
    p.add_argument("--newlib-profile", choices=("portable", "posix"), default="portable",
                   help="portable: strings, integers, sort and a bounded allocator; posix: most of libc "
                        "(stdio, stdlib, time, locale…), leaving errno, sbrk and the system calls to the OS layer")
    p.add_argument("--newlib-cache", type=Path, help="directory for the compiled Newlib IR of the posix profile "
                                                      "(default: $SRC32_CACHE or ~/.cache/llvm-src32)")
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


POSIX_LIBRARY_OPTIMIZATION = "-Os"


def compile_c(clang: str, source: Path, output: Path, args, defines=(), runtime=False, depfile=None) -> None:
    # The bundled string loops are hot in every guest, including -O0 guests.
    # Keep the optional external Newlib profile's compilation policy separate.
    optimization = "-O0" if runtime else args.optimization
    softfloat = runtime and source.is_relative_to(SOFTFLOAT)
    if runtime and (source == ROOT / "runtime" / "string.c" or softfloat):
        optimization = "-O2"
    cxx = source.suffix.lower() in CXX_SUFFIXES
    newlib = getattr(args, "newlib", None)
    posix_library = bool(runtime and newlib and newlib.posix)
    if posix_library and source.is_relative_to(newlib.root):
        # The code size matters more than speed (Foundation runs an image from a 1 MiB buffer).
        optimization = POSIX_LIBRARY_OPTIMIZATION
    if posix_library:
        # Newlib is written for GNU C and builds with the old implicit-declaration rules.
        defines = (*defines, *newlib_posix_defines())
    command = [clang, "--target=mips-unknown-none", "-mabi=o32", "-msoft-float",
               "-ffreestanding", "-fno-builtin", "-ffp-contract=off", "-fno-stack-protector", "-fno-pic", "-fno-pie",
               "-fno-vectorize", "-fno-slp-vectorize", "-fsigned-char", "-G0", "-nostdinc",
               "-std=c++17" if cxx else "-std=gnu11" if posix_library or (newlib and newlib.posix) else "-std=c11",
               optimization, "-S", "-emit-llvm"]
    if posix_library:
        if source.is_relative_to(newlib.root / "newlib" / "libm"):
            command.extend(["-I", str(newlib.root / "newlib" / "libm" / "common")])
        command.extend(["-Wno-error=implicit-function-declaration", "-Wno-error=int-conversion",
                        "-Wno-error=incompatible-pointer-types", "-Wno-error=implicit-int", "-w"])
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
        for forced in args.forced_includes:
            command.extend(["-include", forced])
        for define in args.defines:
            command.extend(["-D", define])
    if softfloat:
        for include in SOFTFLOAT_INCLUDES:
            command.extend(["-I", str(SOFTFLOAT / include)])
        defines = (*defines, *SOFTFLOAT_DEFINES)
    command.extend(f"-D{name}" for name in defines)
    if depfile is not None:
        command.extend(["-MD", "-MF", str(depfile.resolve()), "-MT", "src32_dependency_target"])
    command.extend([str(source.resolve()), "-o", str(output.resolve())])
    result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, errors="replace")
    if result.returncode:
        raise DriverError(f"Clang failed for {source}:\n{result.stderr.strip() or result.stdout.strip()}")


def compile_only(args) -> Path:
    clang = find_clang(args.clang)
    args.newlib = None
    if args.newlib_source:
        from newlib import Newlib
        args.newlib = Newlib(args.newlib_source, clang, args.newlib_heap_size, profile=args.newlib_profile,
                             cache=args.newlib_cache, jobs=1)
    source = args.inputs[0]
    dependency_path = args.depfile.resolve() if args.depfile else None
    compile_c(clang, source, args.output, args, depfile=dependency_path)
    return args.output


def newlib_posix_defines():
    from newlib import POSIX_DEFINES
    return POSIX_DEFINES


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

    if args.compile_only:
        if len(args.inputs) != 1 or args.inputs[0].suffix.lower() not in SOURCE_SUFFIXES:
            raise DriverError("-c takes exactly one C or C++ source")
        if args.output is None:
            args.output = args.inputs[0].with_suffix(".ll")
        elif args.output.suffix.lower() != ".ll":
            raise DriverError("-c writes a .ll file")
        return compile_only(args)
    for source in args.inputs:
        if source.suffix.lower() not in (*SOURCE_SUFFIXES, ".ll", ".bc"):
            raise DriverError(f"unsupported input extension: {source} (expected .c, .cpp, .cc, .cxx, .ll, or .bc)")
        if not source.is_file():
            raise DriverError(f"input file not found: {source}")
    if args.pie and not args.wax3:
        raise DriverError("--pie applies to --wax3 only")
    if args.pie and args.origin:
        raise DriverError("--pie links at a fixed internal address; drop --origin")
    if args.heap_hint and not args.pie:
        raise DriverError("--heap-hint needs --pie")
    if args.heap_hint % 8:
        raise DriverError("--heap-hint must be a multiple of 8")
    if args.pie:
        args.origin = args.stack_top = PIE_BASE
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
    v3 = args.wax3 or args.wal
    if v3:
        if args.protected or args.data_origin is not None:
            raise DriverError("--wax3 and --wal are protected images; drop --protected/--data-origin")
        if args.wal and (args.exports is None or args.origin):
            raise DriverError("--wal needs --exports and is position independent (no --origin)")
        if args.wax3 and args.exports is not None:
            raise DriverError("--exports applies to --wal only")
        if args.wax3 and (not args.user_stack_size or args.user_stack_size % 8
                          or args.user_stack_size > args.stack_top):
            raise DriverError("--user-stack-size must be a nonzero multiple of 8 that fits below --stack-top")
        from v3 import version
        try:
            args.abi = version(args.abi_version)
        except ValueError as exc:
            raise DriverError(f"--abi-version: {exc}") from None
    elif args.import_wal or args.exports or args.image_id:
        raise DriverError("--import-wal, --exports and --image-id need --wax3 or --wal")
    suffix = ".wal" if args.wal else ".wax"
    output = args.output or args.inputs[0].with_suffix(".ll" if args.emit_llvm else ".a" if args.S else suffix)
    if v3 and not (args.emit_llvm or args.S) and output.suffix.lower() != suffix:
        raise DriverError(f"{'--wal' if args.wal else '--wax3'} writes a {suffix} file")
    if args.protected and not (args.emit_llvm or args.S) and output.suffix.lower() != ".wax":
        raise DriverError("--protected needs .wax output: raw .bin cannot carry permissions")
    if not (args.emit_llvm or args.S or v3) and output.suffix.lower() not in (".wax", ".bin"):
        raise DriverError("binary output must have a .wax or .bin extension; use -S for assembly")
    if output.resolve() in {source.resolve() for source in args.inputs}:
        raise DriverError("output must not overwrite an input file")
    if not output.parent.is_dir():
        raise DriverError(f"output directory does not exist: {output.parent}")
    dependencies = {source.resolve() for source in args.inputs}
    dependencies.update(path.resolve() for path in args.import_wal)
    if args.exports:
        dependencies.add(args.exports.resolve())
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
            args.newlib = Newlib(args.newlib_source, clang, args.newlib_heap_size, profile=args.newlib_profile,
                                 cache=args.newlib_cache, jobs=args.jobs if args.jobs > 0 else (os.cpu_count() or 1))
            args.newlib.compile_ir = lambda source, ir: compile_c(clang, source, ir, args, runtime=True)
            dependencies.add(args.newlib.root / "COPYING.NEWLIB")
        jobs = args.jobs if args.jobs > 0 else (os.cpu_count() or 1)
        args.jobs = jobs

        def compile_input(item):
            index, source = item
            parsed_source = source
            dependency_path = None
            if source.suffix.lower() in SOURCE_SUFFIXES:
                parsed_source = work / f"{index:03d}-{source.stem}.ll"
                dependency_path = parsed_source.with_suffix(".d") if args.depfile else None
                compile_c(clang, source, parsed_source, args, depfile=dependency_path)
            return parsed_source, dependency_path

        # Clang runs in separate processes; only the Clang step is parallel.
        # Parsing and linking below stay in input order so the output is identical for any -j.
        with ThreadPoolExecutor(max_workers=jobs) as pool:
            compiled = list(pool.map(compile_input, enumerate(args.inputs)))
        for source, (parsed_source, dependency_path) in zip(args.inputs, compiled):
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
                defined |= {g.name for g in linked.global_variables if not g.is_declaration}
                missing = {f.name for f in linked.functions if f.is_declaration}
                missing |= {g.name for g in linked.global_variables if g.is_declaration}
                missing -= defined
                pending = args.newlib.pending(missing, defined)
                if not pending:
                    break

                def build_newlib(item):
                    name, source, defines, *cached = item
                    if cached:  # the posix profile compiled it into its IR cache already
                        return cached[0], cached[0], None
                    ir = work / f"newlib-{name}.ll"
                    dep = ir.with_suffix(".d") if args.depfile else None
                    # Do not leak application defines/includes into upstream libc.
                    compile_c(clang, source, ir, args, defines=defines, runtime=True, depfile=dep)
                    return source, ir, dep

                with ThreadPoolExecutor(max_workers=args.jobs) as pool:
                    built_newlib = list(pool.map(build_newlib, pending))
                for source, ir, dep in built_newlib:
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
        prunable = link_softfloat(linked, args, clang, work, dependencies, parse_module, check_link_layout)
        ir_text = str(linked)
        if args.save_temps:
            (work / "linked.ll").write_text(ir_text, encoding="utf-8")
        if args.emit_llvm:
            output.write_text(ir_text, encoding="utf-8")
            emit_dependencies()
            return output
        from backend import compile_module
        layout_info = {}
        if v3:
            import v3 as v3_module
            wax3 = load_python(ROOT.parent / "wax" / "wax3.py", "_llvm_src32_wax3")
            layout = Layout(linked)
            image_id = v3_module.check_id(args.image_id or output.stem.lower(), "image ID")
            specs = v3_module.read_exports(args.exports) if args.wal else []
            libraries = []
            for path in args.import_wal:
                try:
                    library = wax3.read_library(path.read_bytes())
                except (OSError, ValueError) as exc:
                    raise DriverError(f"--import-wal {path}: {exc}") from exc
                if library.call_abi != 1 or library.os_id != args.os \
                        or library.os_major != V3_OSES[args.os]["version"][0]:
                    raise DriverError(f"--import-wal {path}: different call ABI or OS ABI")
                libraries.append(library)
            imports = v3_module.resolve_imports(linked, layout, libraries, wax3)
            def generate(origin, stack_top, info):
                return compile_module(linked, origin=origin, stack_top=stack_top,
                                      entry=None if args.wal else args.entry, protected=True,
                                      info=info, pic=args.wal, imports=imports.kinds,
                                      exit_syscall=V3_OSES[args.os]["exit"], page_align=args.page_align,
                                      prunable=prunable)

            assembly = generate(0 if args.wal else args.origin, args.stack_top, layout_info)
        else:
            assembly = compile_module(linked, origin=args.origin, stack_top=args.stack_top, entry=args.entry,
                                      protected=args.protected, info=layout_info, data_origin=args.data_origin,
                                      page_align=args.page_align, prunable=prunable)
        if args.save_temps:
            (work / "output.a").write_text(assembly, encoding="utf-8")
        if args.S:
            output.write_text(assembly, encoding="utf-8")
        else:
            assembler_module = load_python(ROOT.parent / "asm" / "asm.py", "_llvm_src32_assembler")
            wax = output.suffix.lower() == ".wax"
            assembler = assembler_module.Assembler(strip=True, sparse=wax or v3)
            binary = assembler.assemble(assembly, source="<llvm-src32>", base_dir=str(ROOT))
            if v3:
                blocks = protected_blocks(assembler.blocks, layout_info["data_start"])
                options = dict(kind="wal", exports=v3_module.exports_table(specs, linked, layout, layout_info, wax3)) \
                    if args.wal else dict(kind="wax", origin=args.origin, stack_top=args.stack_top,
                                          stack_size=args.user_stack_size)
                if args.pie:
                    # Link again PIE_PROBE higher: what moved is an address (spec_WAX_v3.md §3.1).
                    moved_assembly = generate(args.origin + PIE_PROBE, args.stack_top + PIE_PROBE, {})
                    moved_assembler = assembler_module.Assembler(strip=True, sparse=True)
                    moved_assembler.assemble(moved_assembly, source="<llvm-src32>", base_dir=str(ROOT))
                    moved_blocks = protected_blocks(moved_assembler.blocks, layout_info["data_start"] + PIE_PROBE)
                    try:
                        options["relocs"] = v3_module.relocations(wax3, blocks, moved_blocks, PIE_PROBE)
                    except v3_module.RelocationError as exc:
                        raise DriverError(f"--pie: {exc}") from exc
                    options["heap_hint"] = args.heap_hint
                binary = v3_module.pack(wax3, image_id=image_id, abi=args.abi, blocks=blocks, info=layout_info,
                                        imports=imports, os_id=args.os, os_version=V3_OSES[args.os]["version"],
                                        **options)
            elif wax:
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


def link_softfloat(linked, args, clang, work, dependencies, parse_module, check_link_layout):
    """Link the floating-point runtime when the program needs it.

    Needed are the routines its fp instructions call (fp.py) and any compiler-rt
    / libm routine it declares without defining. Translation units that clash
    with a definition of the program are left out (the program brings its own
    SoftFloat). Returns the names of the runtime functions, which the backend
    emits only when reachable."""
    import fp
    defined = {f.name for f in linked.functions if not f.is_declaration}
    entries = fp.entry_points()
    needed = (fp.module_routines(linked) | ({f.name for f in linked.functions} & entries)) - defined
    if not needed:
        return set()
    clang = clang or find_clang(args.clang)
    owned = defined | {g.name for g in linked.global_variables if not g.is_declaration}
    have = [f"SRC32_HAVE_{name}" for name in sorted(defined & entries)]
    sources = [SOFTFLOAT / "rt.c", *sorted((SOFTFLOAT / "source").glob("*.c")),
               *sorted((SOFTFLOAT / "source" / "8086").glob("*.c"))]

    def build(source):
        ir = work / f"softfloat-{source.stem}.ll"
        dep = ir.with_suffix(".d") if args.depfile else None
        compile_c(clang, source, ir, args, defines=have if source.name == "rt.c" else [], runtime=True, depfile=dep)
        return source, ir, dep

    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        built = list(pool.map(build, sources))
    prunable = set()
    for source, ir, dep in built:
        dependencies.add(source.resolve())
        if dep:
            dependencies.update(read_dependencies(dep))
        module = parse_module(ir)
        functions = {f.name for f in module.functions if not f.is_declaration}
        names = functions | {g.name for g in module.global_variables if not g.is_declaration}
        if names & owned:
            continue
        check_link_layout(linked, module, source)
        try:
            linked.link_in(module)
            linked.verify()
        except RuntimeError as exc:
            raise DriverError(f"linking floating-point runtime {source.name}: {exc}") from exc
        prunable |= functions
        owned |= names
    missing = needed - {f.name for f in linked.functions if not f.is_declaration}
    if missing:
        raise DriverError("floating-point runtime routine(s) not found: " + ", ".join(sorted(missing)))
    return prunable


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
