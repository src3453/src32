"""LLVM-owned layout and deterministic SRC32 big-endian static data.

The input module must already have passed LLVM verification.  No target-dependent
size or struct offset is inferred from printed IR: those come from TargetData.
"""
from __future__ import annotations

from dataclasses import dataclass
import re

from llvmlite import binding as llvm


class CompileError(Exception):
    """An LLVM construct cannot be represented by this freestanding target."""


_NAME = r'(?:"(?:\\.|[^"\\])*"|[-a-zA-Z$._0-9]+)'
_INTEGER = re.compile(r"i([1-9][0-9]*)\Z")


def split_top(text: str, separator: str = ",") -> list[str]:
    """Split outside balanced LLVM delimiters and quoted/escaped names/strings."""
    if not separator:
        raise ValueError("separator cannot be empty")
    if not text.strip():
        return []
    result, stack = [], []
    quote = False
    start = pos = 0
    closing = {"(": ")", "[": "]", "{": "}", "<": ">"}
    while pos < len(text):
        char = text[pos]
        if quote:
            if char == "\\":
                pos += 2
                continue
            if char == '"':
                quote = False
        elif char == '"':
            quote = True
        elif not stack and text.startswith(separator, pos):
            result.append(text[start:pos].strip())
            pos += len(separator)
            start = pos
            continue
        elif char in closing:
            stack.append(closing[char])
        elif char in ")]}>":
            if not stack or stack.pop() != char:
                raise CompileError(f"unbalanced LLVM syntax: {text}")
        pos += 1
    if quote or stack:
        raise CompileError(f"unterminated LLVM syntax: {text}")
    result.append(text[start:].strip())
    return result


def _unquote(name: str) -> str:
    if not name.startswith('"'):
        return name
    raw = _quoted_bytes(name)
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise CompileError("LLVM symbol name is not UTF-8") from exc


def _quoted_bytes(text: str) -> bytes:
    if len(text) < 2 or text[0] != '"' or text[-1] != '"':
        raise CompileError(f"expected LLVM quoted string: {text}")
    result = bytearray()
    pos = 1
    while pos < len(text) - 1:
        if text[pos] == "\\":
            if text[pos + 1:pos + 2] == "\\":
                # LLVM prints a backslash byte as \\, every other escape as \XX.
                result.append(0x5C)
                pos += 2
                continue
            escape = text[pos + 1:pos + 3]
            if len(escape) != 2 or not re.fullmatch(r"[0-9a-fA-F]{2}", escape):
                raise CompileError(f"invalid LLVM byte escape: {text}")
            result.append(int(escape, 16))
            pos += 3
        else:
            result.extend(text[pos].encode("utf-8"))
            pos += 1
    return bytes(result)


_ZERO_INITIALIZERS = ("zeroinitializer", "0", "null", "false")


def _align(value: int, alignment: int) -> int:
    return (value + alignment - 1) & -alignment


def symbol(name: str) -> str:
    """Encode names injectively, including punctuation, without assembler aliases."""
    return "llvm_" + name.encode("utf-8").hex()


def _type_end(text: str) -> int:
    if text.startswith(("[", "{", "<")):
        stack = []
        quote = False
        pairs = {"[": "]", "{": "}", "<": ">", "(": ")"}
        pos = 0
        while pos < len(text):
            char = text[pos]
            if quote:
                if char == "\\":
                    pos += 2
                    continue
                if char == '"':
                    quote = False
            elif char == '"':
                quote = True
            elif char in pairs:
                stack.append(pairs[char])
            elif char in "]}>)":
                if not stack or stack.pop() != char:
                    raise CompileError(f"unbalanced LLVM type: {text}")
                if not stack:
                    return pos + 1
            pos += 1
        raise CompileError(f"unterminated LLVM type: {text}")
    match = re.match(r"%" + _NAME, text)
    if match:
        return match.end()
    match = re.match(r"(?:ptr(?:\s+addrspace\([0-9]+\))?|i[1-9][0-9]*|void|half|bfloat|float|double|fp128|x86_fp80|ppc_fp128|label|metadata|token)(?=\s|$|[,)}\]])", text)
    if not match:
        raise CompileError(f"unsupported LLVM type: {text}")
    return match.end()


def parse_type(text: str, module) -> tuple[object, str]:
    """Read a leading type, using LLVM itself to construct its TypeRef.

    Companion modules/contexts are retained with the source module, so returned
    TypeRefs remain valid.  Named types are taken directly from the input module.
    """
    text = text.strip()
    end = _type_end(text)
    spelling, rest = text[:end], text[end:].lstrip()
    if re.search(r"addrspace\((?!0\))[0-9]+\)", spelling):
        raise CompileError("nonzero address spaces are unsupported")
    if spelling.startswith("%"):
        try:
            return module.get_struct_type(_unquote(spelling[1:])), rest
        except NameError as exc:
            raise CompileError(f"unknown LLVM type {spelling}") from exc
    cache = getattr(module, "_src32_type_cache", None)
    if cache is None:
        cache = {}
        module._src32_type_cache = cache
    if spelling not in cache:
        definitions = "\n".join(str(item) for item in module.struct_types if item.name)
        context = llvm.create_context()
        carrier = ("declare void @__src32_type()" if spelling == "void" else
                   f"@__src32_type = external global {spelling}")
        try:
            owner = llvm.parse_assembly(definitions + "\n" + carrier, context=context)
            if spelling == "void":
                typ = owner.get_function("__src32_type").global_value_type.get_function_return()
            else:
                typ = owner.get_global_variable("__src32_type").global_value_type
        except RuntimeError as exc:
            context.close()
            raise CompileError(f"unsupported LLVM type {spelling}: {exc}") from exc
        cache[spelling] = (typ, owner, context)
    return cache[spelling][0], rest


class Layout:
    def __init__(self, module):
        self.module = module
        specification = str(module.data_layout)
        if not specification or specification.split("-", 1)[0] != "E":
            raise CompileError("SRC32 requires an explicit big-endian LLVM data layout")
        for component in specification.split("-"):
            if component.startswith("ni:"):
                raise CompileError("non-integral pointer layouts are unsupported")
            if re.fullmatch(r"[PGA][0-9]+", component) and int(component[1:]) != 0:
                raise CompileError("nonzero default address spaces are unsupported")
            pointer = re.fullmatch(r"p(?:0)?:([0-9]+):([0-9]+)(?::([0-9]+))?(?::([0-9]+))?", component)
            if pointer and (int(pointer[1]) != 32 or
                            (pointer[4] is not None and int(pointer[4]) != 32)):
                raise CompileError("SRC32 requires 32-bit pointers and pointer indices")
        self.td = llvm.create_target_data(specification)
        ptr, _ = parse_type("ptr", module)
        if self.td.get_abi_size(ptr) != 4 or self.td.get_abi_alignment(ptr) != 4:
            raise CompileError("SRC32 requires 32-bit, four-byte-aligned pointers")
        for width, size in ((8, 1), (16, 2), (32, 4), (64, 8)):
            typ, _ = parse_type(f"i{width}", module)
            if self.td.get_abi_size(typ) != size:
                raise CompileError(f"incompatible i{width} allocation size")

    def _validate(self, typ):
        text = str(typ)
        if re.search(r"addrspace\((?!0\))[0-9]+\)", text):
            raise CompileError("nonzero address spaces are unsupported")
        if typ.is_pointer or _INTEGER.fullmatch(text):
            return
        if text in ("float", "double"):
            return
        if typ.is_array or typ.is_struct:
            if typ.is_struct and typ.is_opaque_struct:
                raise CompileError("opaque structs have no storage layout")
            for child in typ.elements:
                self._validate(child)
            return
        raise CompileError(f"unsupported storage type: {text}")

    def size(self, typ) -> int:
        self._validate(typ)
        return self.td.get_abi_size(typ)

    def align(self, typ) -> int:
        self._validate(typ)
        return self.td.get_abi_alignment(typ)

    def offset(self, typ, index: int) -> int:
        self._validate(typ)
        if not typ.is_struct or not 0 <= index < len(list(typ.elements)):
            raise CompileError(f"invalid struct field index {index} for {typ}")
        return self.td.get_element_offset(typ, index)


@dataclass(frozen=True)
class _Global:
    name: str
    typ: object
    initializer: str
    alignment: int
    constant: bool = False


class DataImage:
    """Two-pass global allocation followed by concrete relocation serialization."""
    def __init__(self, module, base: int = 0):
        self.module = module
        self.layout = Layout(module)
        self.base = base
        self.end = base
        self.symbols: dict[str, int] = {}
        self.writable_base = self.writable_end = self.writable_copy_end = None
        self.globals: list[_Global] = []
        for line in str(module).splitlines():
            if re.match(r"^@" + _NAME + r"\s*=", line):
                body = split_top(line, "=")[1]
                if re.match(r"^(?:[a-z_]+\s+)*(?:alias|ifunc)\b", body):
                    raise CompileError("global aliases and indirect functions are unsupported")
        for value in module.global_variables:
            self._read_global(value)

    def _read_global(self, value):
        if value.linkage.name not in {"external", "internal", "private", "common"}:
            raise CompileError(f"unsupported linkage {value.linkage.name} for @{value.name}")
        if value.storage_class.name != "default":
            raise CompileError(f"DLL storage classes are unsupported for @{value.name}")
        body = split_top(str(value), "=")[1]
        marker = re.search(r"\b(global|constant)\b", body)
        if marker is None:
            raise CompileError(f"unsupported global declaration: {value}")
        qualifiers = body[:marker.start()]
        if re.search(r"\b(thread_local|externally_initialized|comdat)\b", qualifiers):
            raise CompileError(f"unsupported storage semantics for @{value.name}")
        if re.search(r"addrspace\((?!0\))[0-9]+\)", qualifiers):
            raise CompileError("nonzero global address spaces are unsupported")
        parts = split_top(body[marker.end():])
        typ, initializer = parse_type(parts[0], self.module)
        self.layout.size(typ)
        alignment = self.layout.align(typ)
        for attribute in parts[1:]:
            aligned = re.fullmatch(r"align\s+([0-9]+)", attribute)
            if aligned:
                explicit = int(aligned[1])
                if explicit <= 0 or explicit & (explicit - 1):
                    raise CompileError(f"invalid global alignment {explicit}")
                alignment = max(alignment, explicit)
            elif attribute.startswith("!"):
                continue  # Debug/optimization metadata is not stored in the image.
            elif re.fullmatch(r'section\s+"\.(s?data|s?bss|rodata[A-Za-z0-9_.]*)"', attribute):
                continue  # Placement is by the image layout; Newlib puts _impure_ptr in .sdata on MIPS.
            else:
                raise CompileError(f"unsupported global attribute: {attribute}")
        if value.is_declaration:
            return
        if not initializer:
            raise CompileError(f"global @{value.name} has no initializer")
        if value.linkage.name == "common" and initializer != "zeroinitializer" and initializer != "0":
            raise CompileError("common globals must be zero initialized")
        self.globals.append(_Global(value.name, typ, initializer, alignment, marker[1] == "constant"))

    def assign(self, base: int, function_addresses: dict[str, int] | None = None,
               writable_base: int | None = None):
        """Publish all addresses before resolving any initializer (including cycles).

        With `writable_base` (ROM images), LLVM `constant` globals stay at
        `base` while writable globals move to `writable_base`, initialized
        ones first so that the zero-initialized tail can be cleared instead
        of copied; see `writable_layout`.
        """
        if not 0 <= base <= 0xFFFFFFFF:
            raise CompileError("data origin is outside the 32-bit address space")
        self.base = base
        self.writable_base = writable_base
        self.symbols.clear()
        for name, address in (function_addresses or {}).items():
            if not 0 <= address <= 0xFFFFFFFF:
                raise CompileError(f"function @{name} is outside the 32-bit address space")
            self.symbols[name] = address
        if writable_base is None:
            self.end = self._place(self.globals, base)
            self.writable_end = self.writable_copy_end = None
            return self.symbols
        if not 0 <= writable_base <= 0xFFFFFFFF or writable_base % 8:
            raise CompileError("writable data origin must be an 8-byte-aligned 32-bit address")
        self.end = self._place(self.read_only_globals(), base)
        initialized, zeroed = self.writable_globals()
        self.writable_copy_end = _align(self._place(initialized, writable_base), 4)
        self.writable_end = _align(max(self._place(zeroed, self.writable_copy_end), self.writable_copy_end), 4)
        if writable_base < self.end and base < self.writable_end:
            raise CompileError("writable data overlaps the read-only image")
        return self.symbols

    def read_only_globals(self):
        return [item for item in self.globals if self.writable_base is None or item.constant]

    def writable_globals(self):
        writable = [item for item in self.globals if not item.constant]
        # Syntactic test: initializers may reference symbols not yet placed.
        zero = [item for item in writable if item.initializer.strip() in _ZERO_INITIALIZERS]
        return [item for item in writable if item not in zero], zero

    def _place(self, items, position):
        for item in items:
            position = (position + item.alignment - 1) & -item.alignment
            if item.name in self.symbols:
                raise CompileError(f"duplicate function/global symbol @{item.name}")
            if position > 0xFFFFFFFF:
                raise CompileError("static data exceeds the 32-bit address space")
            self.symbols[item.name] = position
            position += self.layout.size(item.typ)
            if position > 0x100000000:
                raise CompileError("static data exceeds the 32-bit address space")
        return position

    def _typed(self, text: str):
        text = text.strip()
        if re.match(r"(?:ptr\b|i[1-9][0-9]*\b|%|\[|\{|<|half\b|bfloat\b|float\b|double\b)", text):
            return parse_type(text, self.module)
        return None, text

    @staticmethod
    def _width(typ) -> int:
        if typ.is_pointer:
            return 32
        match = _INTEGER.fullmatch(str(typ))
        if not match:
            raise CompileError(f"expected integer or pointer type, got {typ}")
        return int(match[1])

    def resolve(self, text: str) -> int:
        """Resolve a typed or untyped scalar LLVM constant against assigned symbols."""
        typ, expression = self._typed(text)
        value = self._resolve_expression(expression)
        if typ is not None:
            self.layout.size(typ)
            value &= (1 << self._width(typ)) - 1
        return value

    def _resolve_expression(self, text: str) -> int:
        text = text.strip()
        if text in {"null", "zeroinitializer", "false", "undef", "poison"}:
            return 0
        if text == "true":
            return 1
        if re.fullmatch(r"[-+]?[0-9]+", text):
            return int(text, 10)
        match = re.fullmatch(r"@(" + _NAME + r")", text)
        if match:
            name = _unquote(match[1])
            try:
                return self.symbols[name]
            except KeyError as exc:
                raise CompileError(f"unresolved symbol @{name}") from exc
        gep = re.fullmatch(r"getelementptr(?:\s+(?:inbounds|nuw|nusw))*\s*\((.*)\)", text, re.S)
        if gep:
            parts = split_top(gep[1])
            if len(parts) < 2:
                raise CompileError(f"malformed constant GEP: {text}")
            element, remainder = parse_type(parts[0], self.module)
            if remainder:
                raise CompileError(f"malformed GEP element type: {parts[0]}")
            address = self.resolve(parts[1])
            current = element
            for position, operand in enumerate(parts[2:]):
                index_type, index_text = parse_type(operand, self.module)
                if not _INTEGER.fullmatch(str(index_type)):
                    raise CompileError("GEP indices must be integers")
                bits = self._width(index_type)
                index = self.resolve(index_text) & ((1 << bits) - 1)
                if index & (1 << (bits - 1)):
                    index -= 1 << bits
                if position == 0:
                    address += index * self.layout.size(current)
                elif current.is_struct:
                    address += self.layout.offset(current, index)
                    current = list(current.elements)[index]
                elif current.is_array:
                    current = list(current.elements)[0]
                    address += index * self.layout.size(current)
                else:
                    raise CompileError(f"GEP indexes into non-aggregate type {current}")
            return address & 0xFFFFFFFF
        arithmetic = re.fullmatch(r"(add|sub)\s*\((.*)\)", text, re.S)
        if arithmetic:
            parts = split_top(arithmetic[2])
            if len(parts) != 2:
                raise CompileError(f"malformed constant arithmetic: {text}")
            left_ty, left = parse_type(parts[0], self.module)
            right_ty, right = parse_type(parts[1], self.module)
            if str(left_ty) != str(right_ty) or not _INTEGER.fullmatch(str(left_ty)):
                raise CompileError(f"constant arithmetic requires matching integers: {text}")
            a, b = self.resolve(left), self.resolve(right)
            value = a + b if arithmetic[1] == "add" else a - b
            return value & ((1 << self._width(left_ty)) - 1)
        cast = re.fullmatch(r"(bitcast|addrspacecast|ptrtoint|inttoptr|trunc|zext|sext)\s*\((.*)\)", text, re.S)
        if cast:
            parts = split_top(cast[2], " to ")
            if len(parts) != 2:
                raise CompileError(f"malformed constant cast: {text}")
            source, operand = parse_type(parts[0], self.module)
            target, rest = parse_type(parts[1], self.module)
            if rest:
                raise CompileError(f"malformed constant cast destination: {text}")
            self.layout.size(source)
            self.layout.size(target)
            source_bits, target_bits = self._width(source), self._width(target)
            value = self.resolve(operand) & ((1 << source_bits) - 1)
            if cast[1] == "sext" and value & (1 << (source_bits - 1)):
                value -= 1 << source_bits
            return value & ((1 << target_bits) - 1)
        raise CompileError(f"unsupported LLVM scalar constant: {text}")

    def constant(self, typ, text: str) -> bytes:
        """Serialize an untyped initializer, including ABI padding, exactly once."""
        size = self.layout.size(typ)
        text = text.strip()
        if text == "zeroinitializer":
            return bytes(size)
        if str(typ) in ("float", "double"):
            from fp import literal_bits
            if text in ("undef", "poison"):
                return bytes(size)
            return literal_bits(typ, text).to_bytes(size, "big")
        integer = _INTEGER.fullmatch(str(typ))
        if integer or typ.is_pointer:
            bits = int(integer[1]) if integer else 32
            value = self.resolve(text) & ((1 << bits) - 1)
            # ABI allocation can exceed the integer's store size (e.g. i24).
            # LLVM big-endian integers occupy their store bytes first; remaining
            # allocation bytes are deterministic padding, not shifted value bits.
            store_size = (bits + 7) // 8
            return value.to_bytes(store_size, "big") + bytes(size - store_size)
        if typ.is_array:
            child = list(typ.elements)[0]
            count = typ.element_count
            if text.startswith('c"'):
                if str(child) != "i8":
                    raise CompileError("LLVM byte strings require an i8 array")
                raw = _quoted_bytes(text[1:])
                if len(raw) != count:
                    raise CompileError("LLVM byte string length differs from array type")
                return raw
            if not (text.startswith("[") and text.endswith("]")):
                raise CompileError(f"unsupported array initializer: {text}")
            values = split_top(text[1:-1])
            if len(values) != count:
                raise CompileError("array initializer element count differs from type")
            return b"".join(self._element(child, value) for value in values)
        if typ.is_struct:
            packed = typ.is_packed_struct
            left, right = ("<{", "}>") if packed else ("{", "}")
            if not (text.startswith(left) and text.endswith(right)):
                raise CompileError(f"unsupported struct initializer: {text}")
            values = split_top(text[len(left):-len(right)])
            fields = list(typ.elements)
            if len(values) != len(fields):
                raise CompileError("struct initializer element count differs from type")
            result = bytearray(size)
            for index, (child, value) in enumerate(zip(fields, values)):
                offset = self.layout.offset(typ, index)
                raw = self._element(child, value)
                result[offset:offset + len(raw)] = raw
            return bytes(result)
        raise CompileError(f"unsupported global initializer type: {typ}")

    def _element(self, expected, text: str) -> bytes:
        actual, initializer = parse_type(text, self.module)
        # Named structs print their definition; equal names are enough here as
        # LLVM has already verified the initializer's complete structural type.
        if str(actual) != str(expected):
            raise CompileError(f"initializer type {actual} does not match {expected}")
        return self.constant(expected, initializer)

    def emit(self, function_addresses: dict[str, int], base: int,
             writable_base: int | None = None) -> list[str]:
        self.assign(base, function_addresses, writable_base)
        result = []
        for item in self.read_only_globals():
            result.extend((f".ORG 0x{self.symbols[item.name]:08X}", f"{symbol(item.name)}:"))
            result.extend(f".BYTE 0x{byte:02X}" for byte in self.constant(item.typ, item.initializer))
        if writable_base is not None:
            # ROM load image of the initialized writable globals, copied by the
            # startup code to [writable_base, writable_copy_end). No labels:
            # references use the RAM addresses resolved by the backend.
            image = bytearray(self.writable_copy_end - writable_base)
            for item in self.writable_globals()[0]:
                offset = self.symbols[item.name] - writable_base
                raw = self.constant(item.typ, item.initializer)
                image[offset:offset + len(raw)] = raw
            result.append(f".ORG 0x{self.load_base:08X}")
            result.extend(f".BYTE 0x{byte:02X}" for byte in image)
        return result

    @property
    def load_base(self) -> int:
        """ROM address of the writable-data load image (ROM layout only)."""
        return _align(self.end, 4)

    @property
    def image_end(self) -> int:
        """First address after everything stored in the output image."""
        if self.writable_base is None:
            return self.end
        return self.load_base + (self.writable_copy_end - self.writable_base)
