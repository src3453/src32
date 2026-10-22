"""WAX/WAL v3 output for the LLVM driver (doc/spec_WAX_v3.md).

The backend emits call ABI 1 code (R30 = module origin, import descriptor
calls, PIC for libraries); this module decides what is imported and
exported, checks C prototypes against the export signatures, and lays out
the v3 file with tools/wax/wax3.py.

The exports of a library are listed in a text file, one per line:

    fnd_write fn(u32,ptr,u32,ptr)->i32
    fnd_version object

Blank lines and text after `#` are ignored. A function is followed by its
signature (spec §6); `object` exports a defined global with its LLVM size.
Imports come from the libraries given with --import-wal: every declared but
undefined function or global that one of them exports is bound to it.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import re

from data import CompileError

FUNCTION, OBJECT = 1, 2
_ID = re.compile(r"[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*\Z")
_SYMBOL = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\Z")
_SIGNATURE = re.compile(r"fn\(((?:u32|i32|u64|i64|ptr|fnptr)(?:,(?:u32|i32|u64|i64|ptr|fnptr))*)?\)"
                        r"->(u32|i32|u64|i64|ptr|fnptr|void)\Z")
# LLVM types of each signature type; signedness is not visible in LLVM IR.
_LLVM = {"u32": "i32", "i32": "i32", "u64": "i64", "i64": "i64", "ptr": "ptr", "void": "void"}


@dataclass
class ExportSpec:
    name: str
    kind: int
    signature: str | None


def check_id(text, what):
    if len(text) > 127 or not _ID.match(text):
        raise CompileError(f"{what} {text!r} must match [a-z][a-z0-9_]*(.[a-z][a-z0-9_]*)* "
                           "and be at most 127 bytes")
    return text


def version(text):
    match = re.fullmatch(r"([0-9]+)\.([0-9]+)", text)
    if not match or not 0 < int(match[1]) <= 0xFFFFFFFF or int(match[2]) > 0xFFFFFFFF:
        raise ValueError("expected MAJOR.MINOR with a nonzero major")
    return int(match[1]), int(match[2])


def read_exports(path: Path) -> list[ExportSpec]:
    result, seen = [], set()
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        words = line.split("#", 1)[0].split()
        if not words:
            continue
        where = f"{path}:{number}"
        if len(words) != 2:
            raise CompileError(f"{where}: expected '<name> <signature>' or '<name> object'")
        name, what = words
        if len(name) > 127 or not _SYMBOL.match(name):
            raise CompileError(f"{where}: invalid symbol name {name!r}")
        if name in seen:
            raise CompileError(f"{where}: {name} is exported twice")
        seen.add(name)
        if what == "object":
            result.append(ExportSpec(name, OBJECT, None))
        else:
            if not _SIGNATURE.match(what):
                raise CompileError(f"{where}: invalid signature {what!r} (WAX v3 §6)")
            result.append(ExportSpec(name, FUNCTION, what))
    if not result:
        raise CompileError(f"{path}: a library must export at least one symbol")
    return result


def check_prototype(function, signature, what):
    """The LLVM function type must be the machine type the signature names."""
    match = _SIGNATURE.match(signature)
    arguments = match[1].split(",") if match[1] else []
    if "fnptr" in arguments or match[2] == "fnptr":
        raise CompileError(f"{what}: fnptr descriptors are not supported by this compiler yet")
    header = str(function).strip().splitlines()[0] if function.is_declaration else \
        next(line for line in str(function).splitlines() if line.startswith("define "))
    if re.search(r"\b(?:sret|byval|inreg)\b", header):
        raise CompileError(f"{what}: aggregates must be passed through explicit pointers (WAX v3 §6)")
    types = [str(a.type) for a in function.arguments]
    result = str(function.global_value_type.get_function_return())
    if function.global_value_type.is_function_vararg:
        raise CompileError(f"{what}: variadic functions cannot be imported or exported")
    expected = [_LLVM[t] for t in arguments]
    if types != expected or result != _LLVM[match[2]]:
        found = f"({', '.join(types)}) -> {result}"
        raise CompileError(f"{what}: the C prototype {found} does not match {signature}; "
                           "interfaces use 32-bit integers, 64-bit integers and pointers only")


@dataclass
class Imports:
    kinds: dict          # name -> "function" / "object", for the backend
    entries: list        # (library index, name, kind, size, signature)
    libraries: list      # wax3.Library of each --import-wal, in order


def resolve_imports(module, layout, libraries, wax3) -> Imports:
    """Bind each undefined symbol of the linked module that a library exports."""
    providers = {}
    for index, library in enumerate(libraries):
        for export in library.exports:
            if export.name in providers:
                other = libraries[providers[export.name][0]].image_id
                raise CompileError(f"{export.name} is exported by both {other} and {library.image_id}")
            providers[export.name] = (index, export)
    kinds, entries = {}, []
    undefined = [(f, True) for f in module.functions
                 if f.is_declaration and not f.name.startswith(("llvm.", "cpt32_"))]
    undefined += [(g, False) for g in module.global_variables if g.is_declaration]
    for value, is_function in sorted(undefined, key=lambda item: item[0].name):
        if value.name not in providers:
            continue
        index, export = providers[value.name]
        what = f"import {value.name} from {libraries[index].image_id}"
        if is_function:
            if export.kind != wax3.FUNCTION:
                raise CompileError(f"{what}: the library exports an object, not a function")
            check_prototype(value, export.signature, what)
            entries.append((index, value.name, wax3.FUNCTION, 0, export.signature))
            kinds[value.name] = "function"
        else:
            if export.kind != wax3.OBJECT:
                raise CompileError(f"{what}: the library exports a function, not an object")
            size = layout.size(value.global_value_type)
            if size != export.size:
                raise CompileError(f"{what}: object size {size} differs from the exported {export.size}")
            entries.append((index, value.name, wax3.OBJECT, size, None))
            kinds[value.name] = "object"
    return Imports(kinds, entries, libraries)


def exports_table(specs, module, layout, info, wax3):
    """Export entries from the backend's function extents and global symbols."""
    definitions = {f.name: f for f in module.functions if not f.is_declaration}
    globals_ = {g.name: g for g in module.global_variables if not g.is_declaration}
    table = []
    for spec in specs:
        if spec.kind == FUNCTION:
            if spec.name not in definitions:
                raise CompileError(f"exported function {spec.name} is not defined")
            check_prototype(definitions[spec.name], spec.signature, f"export {spec.name}")
            start, end = info["extents"][spec.name]
            table.append(wax3.Export(spec.name, wax3.FUNCTION, start, end - start, spec.signature))
        else:
            if spec.name not in globals_:
                raise CompileError(f"exported object {spec.name} is not a defined global")
            size = layout.size(globals_[spec.name].global_value_type)
            if not size:
                raise CompileError(f"exported object {spec.name} has size 0")
            table.append(wax3.Export(spec.name, wax3.OBJECT, info["symbols"][spec.name], size, None))
    return sorted(table, key=lambda e: e.name.encode("ascii"))


def pack(wax3, *, kind, image_id, abi, blocks, info, imports, exports=(), origin=0,
         stack_top=0, stack_size=0, os_id="foundation", os_version=(1, 0)):
    """blocks: (address, data, perm) from the driver, absolute for a WAX and
    image-relative (origin 0) for a WAL."""
    used = sorted({entry[0] for entry in imports.entries})
    dependency = {library: position for position, library in enumerate(used)}
    deps = [wax3.Dependency(imports.libraries[i].image_id, imports.libraries[i].abi_major,
                            imports.libraries[i].abi_minor) for i in used]
    table = [wax3.Import(dependency[index], name, kind_, info["slots"][name] - origin, size, signature)
             for index, name, kind_, size, signature in imports.entries]
    table.sort(key=lambda i: i.slot)
    v3_blocks = [wax3.Block(address, data, perm=perm, alignment=8 if address % 8 == 0 else 4)
                 for address, data, perm in blocks]
    return wax3.build(kind, image_id=image_id, abi_major=abi[0], abi_minor=abi[1], blocks=v3_blocks,
                      entry_offset=0, stack_top=stack_top, stack_size=stack_size, deps=deps,
                      exports=list(exports), imports=table, os_id=os_id, os_major=os_version[0],
                      os_min_minor=os_version[1]).data
