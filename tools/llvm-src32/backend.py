"""Correctness-first, freestanding LLVM integer lowering for the SRC32 ISA.

SSA values live in frame slots.  Only the private stack ABI described here is
used: caller arguments are successive big-endian words at SP, FP points at the
saved FP/LR pair, and scalar results are returned in R1 (low), R2 (high).
"""
from __future__ import annotations

import re
from dataclasses import dataclass

from llvmlite import binding as llvm

from data import CompileError, DataImage, Layout, parse_type, split_top, symbol


def _round(n, a=4):
    return (n + a - 1) // a * a


def _instruction_tail(inst):
    text = str(inst).strip()
    assignment = r'^%(?:"(?:[^"\\]|\\.)*"|[-a-zA-Z$._0-9]+)\s*=\s*'
    text = re.sub(assignment, "", text, count=1)
    return re.sub(r"^" + re.escape(inst.opcode) + r"\s*", "", text, count=1)


def _bits(ty):
    if ty.is_pointer:
        return 32
    if ty.type_kind == llvm.TypeKind.integer:
        return ty.type_width
    return 0


def _mask_identifiers(text):
    text = re.sub(r'"(?:[^"\\]|\\.)*"', '""', text)
    return re.sub(r'[@%](?:"(?:[^"\\]|\\.)*"|[-a-zA-Z$._0-9]+)', "_", text)

@dataclass
class _Fixup:
    line: int
    register: int
    expression: object


class _Code:
    def __init__(self, origin):
        self.lines = [f".ORG 0x{origin:08X}"]
        self.pc = origin
        self.labels = {}
        self.fixups = []
        self.serial = 0

    def emit(self, text):
        self.lines.append("    " + text)
        self.pc += 4
        if self.pc > 0xFFFFFFFF:
            raise CompileError("generated code exceeds the 32-bit address space")

    def label(self, name):
        if name in self.labels:
            raise CompileError(f"duplicate generated label {name}")
        self.labels[name] = self.pc
        self.lines.append(name + ":")

    def fresh(self, hint="local"):
        self.serial += 1
        return f"__llvm_src32_{hint}_{self.serial}"

    def imm(self, reg, value):
        if isinstance(value, int):
            value &= 0xFFFFFFFF
            self.emit(f"LDIH R{reg}, 0x{value >> 16:04X}")
            self.emit(f"LDIL R{reg}, 0x{value & 65535:04X}")
        else:
            self.fixups.append(_Fixup(len(self.lines), reg, value))
            self.emit(f"LDIH R{reg}, 0")
            self.emit(f"LDIL R{reg}, 0")

    def jump(self, label):
        self.imm(12, ("label", label))
        self.emit("JR R12")

    def move(self, dst, src):
        if dst != src:
            self.emit(f"ADD R{dst}, R{src}, R0")

    def add_const(self, dst, src, value, scratch=11):
        if -32768 <= value <= 32767:
            self.emit(f"ADDI R{dst}, R{src}, {value}")
        else:
            self.imm(scratch, value)
            self.emit(f"ADD R{dst}, R{src}, R{scratch}")

    def mem(self, op, reg, base, offset=0):
        if not -32768 <= offset <= 32767:
            self.add_const(11, base, offset, scratch=10)
            base, offset = 11, 0
        self.emit(f"{op} R{reg}, [R{base}+{offset}]")

    def resolve(self, image):
        for fix in self.fixups:
            kind, expression, *extra = fix.expression
            if kind == "label":
                if expression not in self.labels:
                    raise CompileError(f"unresolved function or block {expression}")
                value = self.labels[expression]
            elif kind == "constant":
                value = image.resolve(expression)
                if extra:
                    value >>= extra[0]
            elif kind == "bytes":
                ty, text, offset = expression
                _, untyped = parse_type(text, image.module)
                data = image.constant(ty, untyped)
                value = int.from_bytes(data[offset:offset + 4].ljust(4, b"\0"), "big")
            else:
                raise CompileError(f"internal relocation kind {kind}")
            value &= 0xFFFFFFFF
            self.lines[fix.line] = f"    LDIH R{fix.register}, 0x{value >> 16:04X}"
            self.lines[fix.line + 1] = f"    LDIL R{fix.register}, 0x{value & 65535:04X}"


class _Function:
    def __init__(self, owner, function):
        self.owner, self.fn = owner, function
        self.c, self.layout = owner.code, owner.layout
        self.slots, self.objects, self.shadows = {}, {}, {}
        self.materialized = {}
        self.used = 0
        self.blocks = list(function.blocks)
        self.labels = {b: self.c.fresh("block") for b in self.blocks}
        self.epilogue = self.c.fresh("return")
        self.context = f"function @{function.name}"
        self.check_type(function.global_value_type.get_function_return(), allow_void=True)
        if function.global_value_type.is_function_vararg:
            self.fail("variadic functions are not supported")
        header = next(line for line in str(function).splitlines() if line.startswith("define "))
        attributes = " ".join(a.decode("utf-8") if isinstance(a, bytes) else str(a) for a in function.attributes)
        header += " " + attributes
        header = _mask_identifiers(header)
        if re.search(r"\b(?:(?!(?:ccc|fastcc)\b)[a-zA-Z_0-9]+cc|spir_func|spir_kernel|ptx_kernel|ptx_device|amdgpu_[a-z_]+|cc\s+\d+|naked)\b", header):
            self.fail("unsupported calling convention or naked function")
        # Clang's fastcc and inreg are source-target ABI hints; this whole-program
        # backend remaps both definitions and calls to the private SRC32 stack
        # ABI. Other conventions (and musttail's ABI guarantee) stay unsupported.
        if re.search(r"\b(?:inalloca|preallocated|byref|nest|swiftself|swifterror|swiftasync)\b", header):
            self.fail("unsupported ABI argument attribute")
        if re.search(r"\b(?:personality|prefix|prologue|comdat)\b", header):
            self.fail("unsupported function prefix, prologue, EH personality, or comdat")
        for arg in function.arguments:
            self.check_type(arg.type)
            self.slots[arg] = self.allocate(self.storage(arg.type))
        for block in self.blocks:
            for inst in block.instructions:
                if str(inst.type) != "void":
                    self.check_type(inst.type)
                    self.slots[inst] = self.allocate(self.storage(inst.type))
                if inst.opcode == "phi":
                    self.shadows[inst] = self.allocate(self.storage(inst.type))
                if inst.opcode == "extractvalue":
                    aggregate = list(inst.operands)[0]
                    if aggregate not in self.slots:
                        self.materialized[inst] = self.allocate(self.storage(aggregate.type))
                if inst.opcode == "alloca":
                    ty, _ = parse_type(_instruction_tail(inst), owner.module)
                    self.check_type(ty)
                    if re.search(r"\b(?:inalloca|swifterror)\b", str(inst)):
                        self.fail("inalloca and swifterror allocations are unsupported")
                    align = self.alignment(inst, self.layout.align(ty))
                    count = list(inst.operands)[0]
                    if block == self.blocks[0] and count.is_constant and count.value_kind == llvm.ValueKind.constant_int:
                        n = count.get_constant_value()
                        self.objects[inst] = self.allocate(self.layout.size(ty) * n, align)
        self.frame_size = _round(self.used, 8)
        if self.frame_size > 0x7FFFFFF0:
            self.fail("function frame exceeds the supported 2 GiB range")

    def fail(self, message):
        raise CompileError(f"{self.context}: {message}")

    def check_type(self, ty, allow_void=False):
        if allow_void and str(ty) == "void":
            return
        bits = _bits(ty)
        if bits:
            if bits > 64 or 32 < bits < 64:
                self.fail(f"unsupported integer width i{bits}; supported widths are i1..i32 and i64")
            if ty.is_pointer and "addrspace(" in str(ty):
                self.fail("nonzero pointer address spaces are unsupported")
            return
        if ty.is_struct or ty.is_array:
            for element in ty.elements:
                self.check_type(element)
            return
        self.fail(f"unsupported type {ty} (floating point, vectors, and tokens are not supported)")

    def storage(self, ty):
        return max(4, _round(self.layout.size(ty)))

    def allocate(self, size, alignment=4):
        self.used = _round(self.used + max(size, 1), max(alignment, 4))
        return -self.used

    def alignment(self, inst, default):
        m = re.search(r"\balign\s+(\d+)", str(inst))
        a = int(m.group(1)) if m else default
        if a > 8:
            self.fail(f"stack alignment {a} exceeds the 8-byte ABI alignment")
        return a

    def load(self, value, reg=1, word=0):
        bits = _bits(value.type)
        if value in self.slots:
            self.c.mem("LD", reg, 29, self.slots[value] + 4 * word)
            return
        if value.is_function or value.is_global or value.value_kind in (llvm.ValueKind.function, llvm.ValueKind.global_variable):
            self.c.imm(reg, ("constant", "@" + self.quote_name(value.name)))
            return
        if not value.is_constant:
            self.fail(f"cannot materialize value {value}")
        text = str(value)
        if value.value_kind in (llvm.ValueKind.undef_value, llvm.ValueKind.poison_value):
            # Choosing zero is a valid concrete refinement of undef/poison.
            self.c.imm(reg, 0)
        elif bits:
            shift = 32 if bits > 32 and word == 0 else 0
            if value.value_kind == llvm.ValueKind.constant_int:
                self.c.imm(reg, value.get_constant_value() >> shift)
            else:
                self.c.imm(reg, ("constant", text, shift))
        else:
            self.c.imm(reg, ("bytes", (value.type, text, word * 4)))

    @staticmethod
    def quote_name(name):
        return '"' + name.replace("\\", "\\5C").replace('"', "\\22") + '"'

    def store(self, value, reg=1, word=0):
        self.c.mem("ST", reg, 29, self.slots[value] + word * 4)

    def copy(self, src, dst_offset, ty=None):
        ty = src.type if ty is None else ty
        for w in range(self.storage(ty) // 4):
            self.load(src, 1, w)
            self.c.mem("ST", 1, 29, dst_offset + 4 * w)

    def narrow(self, reg, bits):
        if bits < 32:
            self.c.imm(9, (1 << bits) - 1)
            self.c.emit(f"AND R{reg}, R{reg}, R9")

    def signed(self, reg, bits):
        if bits < 32:
            self.c.imm(9, 32 - bits)
            self.c.emit(f"SLL R{reg}, R{reg}, R9")
            self.c.emit(f"SRA R{reg}, R{reg}, R9")

    def emit(self):
        c = self.c
        c.label(symbol(self.fn.name))
        c.add_const(28, 28, -8)
        c.mem("ST", 29, 28)
        c.mem("ST", 31, 28, 4)
        c.move(29, 28)
        c.add_const(28, 28, -self.frame_size)
        offset = 8
        for arg in self.fn.arguments:
            for w in range(self.storage(arg.type) // 4):
                c.mem("LD", 1, 29, offset + 4 * w)
                self.store(arg, 1, w)
            offset += self.storage(arg.type)
        for block in self.blocks:
            c.label(self.labels[block])
            for inst in block.instructions:
                self.context = f"function @{self.fn.name}, block %{block.name or self.blocks.index(block)}, {str(inst).strip()}"
                self.instruction(inst, block)
        c.label(self.epilogue)
        c.move(28, 29)
        c.mem("LD", 31, 28, 4)
        c.mem("LD", 29, 28)
        c.add_const(28, 28, 8)
        c.emit("JR R31")

    def edge(self, source, target):
        target = self.blocks[self.blocks.index(target)]
        # All reads precede all writes, including cycles such as a<-b,b<-a.
        phis = [i for i in target.instructions if i.opcode == "phi"]
        for phi in phis:
            incoming = list(phi.incoming_blocks)
            try:
                index = incoming.index(source)
            except ValueError:
                self.fail("phi lacks the predecessor edge")
            self.copy(list(phi.operands)[index], self.shadows[phi])
        for phi in phis:
            for w in range(self.storage(phi.type) // 4):
                self.c.mem("LD", 1, 29, self.shadows[phi] + w * 4)
                self.store(phi, 1, w)
        self.c.jump(self.labels[target])

    def instruction(self, inst, block):
        op, args, c = inst.opcode, list(inst.operands), self.c
        if op == "phi":
            return
        if op == "ret":
            if args:
                words = self.storage(args[0].type) // 4
                if words > 2:
                    self.fail("aggregate returns larger than 8 bytes require an explicit sret pointer")
                if words == 2:
                    self.load(args[0], 2, 0)
                    self.load(args[0], 1, 1)
                else:
                    self.load(args[0])
            c.jump(self.epilogue)
        elif op == "br":
            if len(args) == 1:
                self.edge(block, args[0])
            else:
                self.load(args[0])
                other = c.fresh("branch_false")
                c.emit(f"BEQ R1, R0, {other}")
                # Edge copies may be large, so the short branch skips only an absolute jump.
                yes = c.fresh("branch_true")
                c.jump(yes)
                c.label(other)
                self.edge(block, args[1])
                c.label(yes)
                self.edge(block, args[2])
        elif op == "switch":
            self.switch(inst, block)
        elif op == "unreachable":
            # Execution here is undefined; trap rather than accidentally entering another block.
            c.emit("HALT")
            trap = c.fresh("unreachable")
            c.label(trap)
            c.jump(trap)
        elif op == "alloca":
            if inst in self.objects:
                c.add_const(1, 29, self.objects[inst])
            else:
                ty, _ = parse_type(_instruction_tail(inst), self.owner.module)
                if _bits(args[0].type) > 32:
                    self.fail("dynamic alloca count must fit i32")
                self.load(args[0])
                c.imm(2, self.layout.size(ty))
                c.emit("MUL R1, R1, R2")
                c.add_const(1, 1, 7)
                c.imm(2, 0xFFFFFFF8)
                c.emit("AND R1, R1, R2")
                c.emit("SUB R28, R28, R1")
                c.move(1, 28)
            self.store(inst)
        elif op in ("load", "store"):
            operation = _mask_identifiers(str(inst))
            if re.search(r"\b(?:atomic|syncscope)\b", operation):
                self.fail("atomic memory operations are unsupported")
            value, ptr = (inst, args[0]) if op == "load" else (args[0], args[1])
            self.load(ptr, 3)
            bits = _bits(value.type)
            size = self.layout.size(value.type)
            if bits and bits <= 32:
                self.scalar_memory(op, value, 3, bits)
            elif bits:
                for w in range(2):
                    if op == "store":
                        self.load(value, 1, w)
                        c.mem("ST", 1, 3, w * 4)
                    else:
                        c.mem("LD", 1, 3, w * 4)
                        self.store(inst, 1, w)
            else:
                self.aggregate_memory(op, value, 3, size)
        elif op in ("add", "sub", "mul", "sdiv", "udiv", "srem", "urem", "and", "or", "xor", "shl", "lshr", "ashr"):
            self.binary(inst, op, args)
        elif op == "icmp":
            self.compare(inst, args)
        elif op in ("trunc", "zext", "sext", "ptrtoint", "inttoptr", "bitcast", "addrspacecast"):
            self.cast(inst, args[0], op)
        elif op == "getelementptr":
            self.gep(inst, args)
        elif op == "select":
            self.load(args[0])
            no, done = c.fresh("select_no"), c.fresh("select_done")
            yes = c.fresh("select_yes")
            c.emit(f"BNE R1, R0, {yes}")
            c.jump(no)
            c.label(yes)
            for w in range(self.storage(inst.type) // 4):
                self.load(args[1], 1, w)
                self.store(inst, 1, w)
            c.jump(done)
            c.label(no)
            for w in range(self.storage(inst.type) // 4):
                self.load(args[2], 1, w)
                self.store(inst, 1, w)
            c.label(done)
        elif op == "call":
            self.call(inst, args)
        elif op in ("extractvalue", "insertvalue"):
            self.aggregate_value(inst, args, op)
        elif op == "freeze":
            self.copy(args[0], self.slots[inst])
        else:
            self.fail(f"unsupported LLVM instruction {op}")

    def scalar_memory(self, op, value, base, bits):
        c = self.c
        size = (bits + 7) // 8
        mnemonic = {1: "B", 2: "H", 4: ""}.get(size)
        if op == "store":
            self.load(value)
            if mnemonic is not None:
                c.mem("ST" + mnemonic, 1, base)
            else:
                for j in range(size):
                    c.imm(9, 8 * (size - j - 1))
                    c.emit("SRL R2, R1, R9")
                    c.mem("STB", 2, base, j)
        else:
            if mnemonic is not None:
                c.mem("LD" + mnemonic, 1, base)
            else:
                c.move(1, 0)
                for j in range(size):
                    c.mem("LDB", 2, base, j)
                    c.imm(9, 8 * (size - j - 1))
                    c.emit("SLL R2, R2, R9")
                    c.emit("OR R1, R1, R2")
            self.narrow(1, bits)
            self.store(value)

    def aggregate_memory(self, op, value, base, size):
        c = self.c
        for offset in range(0, size, 4):
            count = min(4, size - offset)
            if op == "store":
                self.load(value, 1, offset // 4)
                if count == 4:
                    c.mem("ST", 1, base, offset)
                else:
                    for j in range(count):
                        c.imm(9, 24 - 8 * j)
                        c.emit("SRL R2, R1, R9")
                        c.mem("STB", 2, base, offset + j)
            else:
                if count == 4:
                    c.mem("LD", 1, base, offset)
                else:
                    c.imm(1, 0)
                    for j in range(count):
                        c.mem("LDB", 2, base, offset + j)
                        c.imm(9, 24 - 8 * j)
                        c.emit("SLL R2, R2, R9")
                        c.emit("OR R1, R1, R2")
                self.store(value, 1, offset // 4)

    def binary(self, inst, op, args):
        bits, c = _bits(inst.type), self.c
        if bits > 32:
            self.wide_binary(inst, op, args)
            return
        self.load(args[0], 1)
        self.load(args[1], 2)
        if op in ("sdiv", "srem", "ashr"):
            self.signed(1, bits)
        if op in ("sdiv", "srem"):
            self.signed(2, bits)
        mnemonics = {"add": "ADD", "sub": "SUB", "mul": "MUL", "sdiv": "DIV", "udiv": "DIVU", "srem": "MOD", "and": "AND", "or": "OR", "xor": "XOR", "shl": "SLL", "lshr": "SRL", "ashr": "SRA"}
        if op == "urem":
            c.emit("DIVU R3, R1, R2")
            c.emit("MUL R3, R3, R2")
            c.emit("SUB R1, R1, R3")
        else:
            c.emit(f"{mnemonics[op]} R1, R1, R2")
        self.narrow(1, bits)
        self.store(inst)

    def wide_binary(self, inst, op, args):
        c = self.c
        self.load(args[0], 2, 0)
        self.load(args[0], 1, 1)
        self.load(args[1], 4, 0)
        self.load(args[1], 3, 1)
        if op == "add":
            c.emit("ADD R5, R1, R3")
            c.emit("SLTU R6, R5, R1")
            c.emit("ADD R2, R2, R4")
            c.emit("ADD R2, R2, R6")
            c.move(1, 5)
        elif op == "sub":
            c.emit("SLTU R6, R1, R3")
            c.emit("SUB R1, R1, R3")
            c.emit("SUB R2, R2, R4")
            c.emit("SUB R2, R2, R6")
        elif op in ("and", "or", "xor"):
            c.emit(f"{op.upper()} R1, R1, R3")
            c.emit(f"{op.upper()} R2, R2, R4")
        elif op == "mul":
            c.emit("MULH R5, R1, R3")
            c.imm(9, 31)
            c.emit("SRA R6, R1, R9")
            c.emit("AND R6, R6, R3")
            c.emit("ADD R5, R5, R6")
            c.emit("SRA R6, R3, R9")
            c.emit("AND R6, R6, R1")
            c.emit("ADD R5, R5, R6")
            c.emit("MUL R2, R2, R3")
            c.emit("MUL R4, R4, R1")
            c.emit("ADD R2, R2, R4")
            c.emit("ADD R2, R2, R5")
            c.emit("MUL R1, R1, R3")
        elif op in ("shl", "lshr", "ashr"):
            self.wide_shift(op)
        elif op in ("udiv", "urem", "sdiv", "srem"):
            self.wide_divide(op)
        else:
            self.fail(f"unsupported i64 operation {op}")
        self.store(inst, 2, 0)
        self.store(inst, 1, 1)

    def wide_shift(self, op):
        c = self.c
        done, large = c.fresh("shift_done"), c.fresh("shift_large")
        c.emit(f"BEQ R3, R0, {done}")
        c.imm(5, 32)
        c.emit("SLTU R6, R3, R5")
        c.emit(f"BEQ R6, R0, {large}")
        c.emit("SUB R5, R5, R3")
        if op == "shl":
            c.emit("SRL R6, R1, R5")
            c.emit("SLL R2, R2, R3")
            c.emit("OR R2, R2, R6")
            c.emit("SLL R1, R1, R3")
        else:
            c.emit("SLL R6, R2, R5")
            c.emit("SRL R1, R1, R3")
            c.emit("OR R1, R1, R6")
            c.emit(f"{'SRA' if op == 'ashr' else 'SRL'} R2, R2, R3")
        c.emit(f"JMP {done}")
        c.label(large)
        c.add_const(3, 3, -32)
        if op == "shl":
            c.emit("SLL R2, R1, R3")
            c.move(1, 0)
        else:
            c.emit(f"{'SRA' if op == 'ashr' else 'SRL'} R1, R2, R3")
            if op == "ashr":
                c.imm(9, 31)
                c.emit("SRA R2, R2, R9")
            else:
                c.move(2, 0)
        c.label(done)

    def negate_pair(self, low, high):
        c = self.c
        c.emit(f"SLTU R8, R0, R{low}")
        c.emit(f"SUB R{low}, R0, R{low}")
        c.emit(f"SUB R{high}, R0, R{high}")
        c.emit(f"SUB R{high}, R{high}, R8")

    def wide_divide(self, op):
        c = self.c
        signed = op.startswith("s")
        if signed:
            c.imm(9, 31)
            c.emit("SRL R12, R2, R9")
            c.emit("SRL R11, R4, R9")
            c.emit("XOR R11, R11, R12")
            a, b = c.fresh("div_abs_a"), c.fresh("div_abs_b")
            c.emit(f"BEQ R12, R0, {a}")
            self.negate_pair(1, 2)
            c.label(a)
            c.emit("SLT R8, R4, R0")
            c.emit(f"BEQ R8, R0, {b}")
            self.negate_pair(3, 4)
            c.label(b)
        c.move(5, 0)
        c.move(6, 0)
        c.imm(7, 64)
        loop, subtract, skip = c.fresh("div_loop"), c.fresh("div_sub"), c.fresh("div_skip")
        c.label(loop)
        c.imm(9, 31)
        c.emit("SRL R10, R6, R9")  # 65th remainder bit
        c.emit("SRL R8, R5, R9")
        c.imm(9, 1)
        c.emit("SLL R6, R6, R9")
        c.emit("OR R6, R6, R8")
        c.imm(9, 31)
        c.emit("SRL R8, R2, R9")
        c.imm(9, 1)
        c.emit("SLL R5, R5, R9")
        c.emit("OR R5, R5, R8")
        c.imm(9, 31)
        c.emit("SRL R8, R1, R9")
        c.imm(9, 1)
        c.emit("SLL R2, R2, R9")
        c.emit("OR R2, R2, R8")
        c.emit("SLL R1, R1, R9")
        c.emit(f"BNE R10, R0, {subtract}")
        c.emit("SLTU R8, R6, R4")
        c.emit(f"BNE R8, R0, {skip}")
        c.emit("SLTU R8, R4, R6")
        c.emit(f"BNE R8, R0, {subtract}")
        c.emit("SLTU R8, R5, R3")
        c.emit(f"BNE R8, R0, {skip}")
        c.label(subtract)
        c.emit("SLTU R8, R5, R3")
        c.emit("SUB R5, R5, R3")
        c.emit("SUB R6, R6, R4")
        c.emit("SUB R6, R6, R8")
        c.add_const(1, 1, 1)
        c.label(skip)
        c.add_const(7, 7, -1)
        c.emit(f"BNE R7, R0, {loop}")
        if op.endswith("rem"):
            c.move(1, 5)
            c.move(2, 6)
        if signed:
            done = c.fresh("div_sign_done")
            sign = 12 if op == "srem" else 11
            c.emit(f"BEQ R{sign}, R0, {done}")
            self.negate_pair(1, 2)
            c.label(done)

    def compare(self, inst, args):
        c = self.c
        predicate = _instruction_tail(inst).split()
        pred = predicate[1] if predicate[0] == "samesign" else predicate[0]
        if pred not in {"eq", "ne", "slt", "sle", "sgt", "sge", "ult", "ule", "ugt", "uge"}:
            self.fail(f"unsupported comparison predicate {pred}")
        bits = _bits(args[0].type)
        wide = bits > 32
        self.load(args[0], 1, 1 if wide else 0)
        self.load(args[1], 3, 1 if wide else 0)
        if wide:
            self.load(args[0], 2, 0)
            self.load(args[1], 4, 0)
        if pred in ("eq", "ne"):
            c.emit("XOR R1, R1, R3")
            if wide:
                c.emit("XOR R2, R2, R4")
                c.emit("OR R1, R1, R2")
            c.emit("SLTU R1, R0, R1")
            if pred == "eq":
                c.imm(9, 1)
                c.emit("XOR R1, R1, R9")
        else:
            signed = pred.startswith("s")
            swap = pred[1:] in ("gt", "le")
            invert = pred[1:] in ("ge", "le")
            if not wide and signed:
                self.signed(1, bits)
                self.signed(3, bits)
            a, b = (3, 1) if swap else (1, 3)
            if wide:
                ah, bh = (4, 2) if swap else (2, 4)
                c.emit(f"SLTU R5, R{a}, R{b}")
                c.emit(f"XOR R6, R{ah}, R{bh}")
                c.emit("SLTU R6, R0, R6")
                c.imm(9, 1)
                c.emit("XOR R6, R6, R9")
                c.emit("AND R5, R5, R6")
                c.emit(f"{'SLT' if signed else 'SLTU'} R1, R{ah}, R{bh}")
                c.emit("OR R1, R1, R5")
            else:
                c.emit(f"{'SLT' if signed else 'SLTU'} R1, R{a}, R{b}")
            if invert:
                c.imm(9, 1)
                c.emit("XOR R1, R1, R9")
        self.store(inst)

    def cast(self, inst, source, op):
        c = self.c
        src, dst = _bits(source.type), _bits(inst.type)
        if not src or not dst:
            self.fail("aggregate bitcasts are unsupported")
        if "addrspace(" in str(source.type) or "addrspace(" in str(inst.type):
            self.fail("nonzero address spaces are unsupported")
        self.load(source, 1, 1 if src > 32 else 0)
        if op == "sext":
            self.signed(1, min(src, 32))
        if dst > 32:
            if src > 32:
                self.load(source, 2, 0)
            elif op == "sext":
                c.imm(9, 31)
                c.emit("SRA R2, R1, R9")
            else:
                c.move(2, 0)
            self.store(inst, 2, 0)
            self.store(inst, 1, 1)
        else:
            self.narrow(1, dst)
            self.store(inst)

    def gep(self, inst, args):
        c = self.c
        text = _instruction_tail(inst)
        text = re.sub(r"^(?:(?:inbounds|nuw|nusw)\s+)+", "", text)
        ty, _ = parse_type(text, self.owner.module)
        self.load(args[0], 3)
        for index, value in enumerate(args[1:]):
            if index > 0 and ty.is_struct:
                if value.value_kind != llvm.ValueKind.constant_int:
                    self.fail("GEP struct index must be constant")
                field = value.get_constant_value()
                c.add_const(3, 3, self.layout.offset(ty, field))
                ty = list(ty.elements)[field]
                continue
            if index > 0:
                if not ty.is_array:
                    self.fail(f"GEP cannot index through {ty}")
                ty = list(ty.elements)[0]
            self.load(value, 1, 1 if _bits(value.type) > 32 else 0)
            self.signed(1, min(_bits(value.type), 32))
            c.imm(2, self.layout.size(ty))
            c.emit("MUL R1, R1, R2")
            c.emit("ADD R3, R3, R1")
        self.store(inst, 3)

    def switch(self, inst, block):
        c, args = self.c, list(inst.operands)
        # LLVM's C operand iterator does not reliably expose switch case constants.
        # Parse only this small, verified textual instruction grammar.
        text = str(inst)
        case_list = re.search(r"\[(.*)\]", text, re.S)
        case_text = case_list[1] if case_list else ""
        case_pattern = re.compile(
            r"(i\d+)\s+(true|false|-?\d+)\s*,\s*label\s+"
            r"(%(?:\"(?:[^\"\\]|\\.)*\"|[-a-zA-Z$._0-9]+))")
        cases = list(case_pattern.finditer(case_text))
        if re.sub(case_pattern, "", case_text).strip():
            self.fail(f"unsupported switch case syntax: {case_text}")
        cases = [match.groups() for match in cases]
        labels = {"%" + self.quote_name(b.name): b for b in self.blocks if b.name}
        labels.update({"%" + b.name: b for b in self.blocks if b.name})
        # Unnamed blocks have empty ValueRef names; recover their printed slot label.
        for b in self.blocks:
            m = re.search(r"^\s*([-a-zA-Z$._0-9]+|\".*?\"):", str(b), re.M)
            if m:
                labels["%" + m.group(1)] = b
        bits = _bits(args[0].type)
        destinations = []
        for ty, literal, target in cases:
            if target not in labels:
                self.fail(f"cannot resolve switch destination {target}")
            label, next_case = c.fresh("switch_case"), c.fresh("switch_next")
            self.load(args[0], 1, 1 if bits > 32 else 0)
            value = (1 if literal == "true" else 0 if literal == "false" else int(literal))
            value &= (1 << bits) - 1
            c.imm(2, value)
            c.emit(f"BNE R1, R2, {next_case}")
            if bits > 32:
                self.load(args[0], 1, 0)
                c.imm(2, value >> 32)
                c.emit(f"BNE R1, R2, {next_case}")
            c.jump(label)
            c.label(next_case)
            destinations.append((label, labels[target]))
        self.edge(block, args[1])
        for label, target in destinations:
            c.label(label)
            self.edge(block, target)

    def aggregate_value(self, inst, args, op):
        c = self.c
        fields = split_top(_instruction_tail(inst))
        indices = [int(x.strip()) for x in fields[2 if op == "insertvalue" else 1:]]
        ty, offset = args[0].type, 0
        for index in indices:
            if ty.is_struct:
                offset += self.layout.offset(ty, index)
                ty = list(ty.elements)[index]
            elif ty.is_array:
                ty = list(ty.elements)[0]
                offset += self.layout.size(ty) * index
            else:
                self.fail("invalid aggregate value index")
        if op == "insertvalue":
            self.copy(args[0], self.slots[inst])
            self.load(args[1], 1, 0)
            c.add_const(3, 29, self.slots[inst] + offset)
            bits = _bits(ty)
            if bits and bits <= 32:
                self.scalar_memory("store", args[1], 3, bits)
            else:
                self.aggregate_memory("store", args[1], 3, self.layout.size(ty))
        else:
            # An aggregate constant need not already occupy a frame slot.
            if args[0] not in self.slots:
                base = self.materialized[inst]
                self.copy(args[0], base)
            else:
                base = self.slots[args[0]]
            c.add_const(3, 29, base + offset)
            bits = _bits(ty)
            if bits and bits <= 32:
                self.scalar_memory("load", inst, 3, bits)
            else:
                self.aggregate_memory("load", inst, 3, self.layout.size(ty))

    def call(self, inst, args):
        c, target, values = self.c, args[-1], args[:-1]
        name = target.name if target.value_kind == llvm.ValueKind.function else ""
        text = str(inst)
        syntax = _mask_identifiers(text)
        if re.search(r"\b(?:musttail|(?!(?:ccc|fastcc)\b)[a-zA-Z_0-9]+cc|spir_func|spir_kernel|ptx_kernel|ptx_device|amdgpu_[a-z_]+|cc\s+\d+)\b", syntax):
            self.fail("unsupported call convention or musttail call")
        if "..." in syntax:
            self.fail("variadic calls are unsupported")
        if re.search(r"\b(?:inalloca|preallocated|byref|nest|swiftself|swifterror|swiftasync)\b", syntax):
            self.fail("unsupported ABI call argument attribute")
        if re.search(r"\[\s*\"", text):
            self.fail("call operand bundles are unsupported")
        if name == "cpt32_halt":
            if values or str(inst.type) != "void":
                self.fail("cpt32_halt must be called with no arguments and no return value")
            c.emit("HALT")
            return
        if name.startswith("llvm."):
            self.intrinsic(inst, name, values)
            return
        if name:
            if name not in self.owner.definitions:
                self.fail(f"undefined external function @{name}; freestanding modules must provide a definition")
            if self.owner.definitions[name].global_value_type.is_function_vararg:
                self.fail("variadic calls are unsupported")
        if str(inst.type) != "void" and self.storage(inst.type) > 8:
            self.fail("aggregate call results larger than 8 bytes require an sret pointer")
        # LLVM byval means the callee may mutate a private caller-created copy.
        # Locate byval types in the textual parameter list, preserving nesting.
        byvals = {}
        call_args = self.call_argument_text(text)
        for index, part in enumerate(call_args):
            m = re.search(r"\bbyval\(", part)
            if m:
                ty, rest = parse_type(part[m.end():], self.owner.module)
                if not rest.lstrip().startswith(")"):
                    self.fail("malformed byval type")
                byvals[index] = ty
        arg_bytes = sum(self.storage(v.type) for v in values)
        total, copies = arg_bytes, {}
        for index, ty in byvals.items():
            total = _round(total, min(8, self.layout.align(ty)))
            copies[index] = total
            total += self.layout.size(ty)
        total = _round(total, 8)
        c.add_const(28, 28, -total)
        offset = 0
        for index, value in enumerate(values):
            if index in copies:
                self.load(value, 3)
                c.add_const(4, 28, copies[index])
                self.byte_copy(self.layout.size(byvals[index]))
                c.add_const(1, 28, copies[index])
                c.mem("ST", 1, 28, offset)
            else:
                for word in range(self.storage(value.type) // 4):
                    self.load(value, 1, word)
                    c.mem("ST", 1, 28, offset + word * 4)
            offset += self.storage(value.type)
        self.load(target, 12)
        c.emit("JALR R12")
        c.add_const(28, 28, total)
        if str(inst.type) != "void":
            if self.storage(inst.type) > 4:
                self.store(inst, 2, 0)
                self.store(inst, 1, 1)
            else:
                self.narrow(1, _bits(inst.type) or 32)
                self.store(inst)

    @staticmethod
    def call_argument_text(text):
        # The final balanced (...) group before call attributes is the argument list.
        groups, stack, quoted, escaped = [], [], False, False
        for index, char in enumerate(text):
            if quoted:
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == '"':
                    quoted = False
            elif char == '"':
                quoted = True
            elif char == "(":
                stack.append(index)
            elif char == ")" and stack:
                start = stack.pop()
                if not stack:
                    groups.append(text[start + 1:index])
        return split_top(groups[-1]) if groups else []

    def byte_copy(self, length):
        # R3 source, R4 destination. Constant-sized byval copies have no helper ABI.
        c = self.c
        c.imm(5, length)
        self.memory_loop("memcpy")

    def memory_loop(self, kind):
        # R3 source/value, R4 destination, R5 length. All registers caller-saved.
        c = self.c
        done, loop = c.fresh("memory_done"), c.fresh("memory_loop")
        c.emit(f"BEQ R5, R0, {done}")
        if kind == "memmove":
            forward = c.fresh("memory_forward")
            c.emit("SLTU R6, R3, R4")
            c.emit(f"BEQ R6, R0, {forward}")
            c.emit("ADD R3, R3, R5")
            c.emit("ADD R4, R4, R5")
            backward = c.fresh("memory_backward")
            c.label(backward)
            c.add_const(3, 3, -1)
            c.add_const(4, 4, -1)
            c.mem("LDB", 1, 3)
            c.mem("STB", 1, 4)
            c.add_const(5, 5, -1)
            c.emit(f"BNE R5, R0, {backward}")
            c.emit(f"JMP {done}")
            c.label(forward)
        c.label(loop)
        if kind == "memset":
            c.mem("STB", 3, 4)
        else:
            c.mem("LDB", 1, 3)
            c.mem("STB", 1, 4)
            c.add_const(3, 3, 1)
        c.add_const(4, 4, 1)
        c.add_const(5, 5, -1)
        c.emit(f"BNE R5, R0, {loop}")
        c.label(done)

    def integer_intrinsic(self, inst, name, values):
        c, bits = self.c, _bits(inst.type)
        kind = name.split(".")[1]
        wide = bits > 32
        if kind in ("smin", "smax", "umin", "umax"):
            self.load(values[0], 1, 1 if wide else 0)
            self.load(values[1], 3, 1 if wide else 0)
            signed = kind.startswith("s")
            if wide:
                self.load(values[0], 2, 0)
                self.load(values[1], 4, 0)
                c.emit("SLTU R5, R1, R3")
                c.emit("XOR R6, R2, R4")
                c.emit("SLTU R6, R0, R6")
                c.imm(9, 1)
                c.emit("XOR R6, R6, R9")
                c.emit("AND R5, R5, R6")
                c.emit(f"{'SLT' if signed else 'SLTU'} R1, R2, R4")
                c.emit("OR R1, R1, R5")
            else:
                if signed:
                    self.signed(1, bits)
                    self.signed(3, bits)
                c.emit(f"{'SLT' if signed else 'SLTU'} R1, R1, R3")
            second, done = c.fresh("minmax_second"), c.fresh("minmax_done")
            c.emit(f"{'BEQ' if kind.endswith('min') else 'BNE'} R1, R0, {second}")
            self.copy(values[0], self.slots[inst])
            c.emit(f"JMP {done}")
            c.label(second)
            self.copy(values[1], self.slots[inst])
            c.label(done)
            return
        self.load(values[0], 1, 1 if wide else 0)
        if wide:
            self.load(values[0], 2, 0)
        if kind == "abs":
            if not wide:
                self.signed(1, bits)
            c.emit(f"SLT R3, R{2 if wide else 1}, R0")
            done = c.fresh("abs_done")
            c.emit(f"BEQ R3, R0, {done}")
            if wide:
                self.negate_pair(1, 2)
            else:
                c.emit("SUB R1, R0, R1")
            c.label(done)
            if wide:
                self.store(inst, 2, 0)
                self.store(inst, 1, 1)
            else:
                self.narrow(1, bits)
                self.store(inst)
            return
        if kind == "bswap":
            if bits not in (16, 32, 64):
                self.fail(f"unsupported bswap width {bits}")
            for output_word in range(2 if wide else 1):
                source_word = 1 - output_word if wide else 0
                self.load(values[0], 1, source_word)
                c.move(3, 0)
                n = 4 if wide else bits // 8
                for byte in range(n):
                    c.imm(9, byte * 8)
                    c.emit("SRL R4, R1, R9")
                    c.imm(9, 255)
                    c.emit("AND R4, R4, R9")
                    c.imm(9, (n - byte - 1) * 8)
                    c.emit("SLL R4, R4, R9")
                    c.emit("OR R3, R3, R4")
                self.store(inst, 3, output_word)
            return
        if kind not in ("ctpop", "ctlz", "cttz"):
            self.fail(f"unsupported integer intrinsic @{name}")
        c.move(5, 0)
        c.imm(6, bits)
        loop, done = c.fresh("bitcount_loop"), c.fresh("bitcount_done")
        c.label(loop)
        c.imm(9, (1 << (31 if wide else bits - 1)) if kind == "ctlz" else 1)
        c.emit(f"AND R3, R{2 if wide and kind == 'ctlz' else 1}, R9")
        if kind == "ctpop":
            c.emit("ADD R5, R5, R3")
        else:
            c.emit(f"BNE R3, R0, {done}")
            c.add_const(5, 5, 1)
        if wide:
            if kind == "ctlz":
                c.imm(9, 31)
                c.emit("SRL R3, R1, R9")
                c.imm(9, 1)
                c.emit("SLL R2, R2, R9")
                c.emit("OR R2, R2, R3")
                c.emit("SLL R1, R1, R9")
            else:
                c.imm(9, 31)
                c.emit("SLL R3, R2, R9")
                c.imm(9, 1)
                c.emit("SRL R1, R1, R9")
                c.emit("OR R1, R1, R3")
                c.emit("SRL R2, R2, R9")
        else:
            c.imm(9, 1)
            c.emit(f"{'SLL' if kind == 'ctlz' else 'SRL'} R1, R1, R9")
        c.add_const(6, 6, -1)
        c.emit(f"BNE R6, R0, {loop}")
        c.label(done)
        if wide:
            self.store(inst, 0, 0)
            self.store(inst, 5, 1)
        else:
            self.store(inst, 5)

    def intrinsic(self, inst, name, values):
        c = self.c
        if name.startswith("llvm.usub.sat."):
            bits = _bits(inst.type)
            wide = bits > 32
            self.load(values[0], 1, 1 if wide else 0)
            self.load(values[1], 3, 1 if wide else 0)
            if not wide:
                self.narrow(1, bits)
                self.narrow(3, bits)
            c.emit("SLTU R6, R1, R3")  # low-word borrow
            if wide:
                self.load(values[0], 2, 0)
                self.load(values[1], 4, 0)
                # Underflow = high(a)<high(b) || (high(a)==high(b) && borrow).
                c.emit("XOR R5, R2, R4")
                c.emit("SLTU R5, R0, R5")
                c.imm(9, 1)
                c.emit("XOR R5, R5, R9")
                c.emit("AND R5, R5, R6")
                c.emit("SLTU R7, R2, R4")
                c.emit("OR R5, R5, R7")
                c.emit("SUB R2, R2, R4")
                c.emit("SUB R2, R2, R6")
            else:
                c.move(5, 6)
            c.emit("SUB R1, R1, R3")
            c.add_const(5, 5, -1)  # no underflow: all ones; underflow: zero
            c.emit("AND R1, R1, R5")
            if wide:
                c.emit("AND R2, R2, R5")
                self.store(inst, 2, 0)
                self.store(inst, 1, 1)
            else:
                self.narrow(1, bits)
                self.store(inst)
            return
        if name.split(".")[1] in ("smin", "smax", "umin", "umax", "abs", "bswap", "ctpop", "ctlz", "cttz"):
            self.integer_intrinsic(inst, name, values)
            return
        if name.startswith(("llvm.lifetime.", "llvm.dbg.")):
            return
        if name.startswith("llvm.stacksave"):
            self.store(inst, 28)
            return
        if name.startswith("llvm.stackrestore"):
            self.load(values[0], 28)
            return
        memory = next((kind for kind in ("memcpy", "memmove", "memset") if name.startswith("llvm." + kind + ".")), None)
        if memory:
            if ".atomic." in name:
                self.fail("atomic memory intrinsics are unsupported")
            self.load(values[0], 4)
            self.load(values[1], 3)
            if _bits(values[2].type) > 32:
                # SRC32 pointers and object sizes are 32-bit; an i64 length must be known to fit.
                if values[2].value_kind != llvm.ValueKind.constant_int or values[2].get_constant_value() > 0xFFFFFFFF:
                    self.fail("memory intrinsic i64 length must be a constant fitting 32 bits")
                self.load(values[2], 5, 1)
            else:
                self.load(values[2], 5)
            self.memory_loop(memory)
            return
        if name.startswith("llvm.expect."):
            self.copy(values[0], self.slots[inst])
            return
        if name.startswith("llvm.assume"):
            # assume has no runtime side effect; invalid assumptions already imply UB.
            return
        self.fail(f"unsupported LLVM intrinsic @{name}")


class _Module:
    def __init__(self, module, origin, stack_top, entry):
        self.module = module
        self.layout = Layout(module)
        self.image = DataImage(module)
        self.code = _Code(origin)
        self.definitions = {f.name: f for f in module.functions if not f.is_declaration}
        self.origin, self.stack_top, self.entry = origin, stack_top, entry

    def compile(self):
        c = self.code
        if "cpt32_halt" in self.definitions:
            raise CompileError("function @cpt32_halt is reserved for the SRC32 HALT intrinsic")
        entry = self.definitions[self.entry]
        if list(entry.arguments):
            raise CompileError("freestanding entry must take no arguments (use int main(void))")
        result = entry.global_value_type.get_function_return()
        if str(result) not in ("void", "i32"):
            raise CompileError("freestanding entry must return i32 or void")
        c.label("__llvm_src32_start")
        c.imm(28, self.stack_top)
        c.move(29, 0)
        irq_dispatch = self.definitions.get("irqc_dispatch")
        if irq_dispatch is not None:
            if list(irq_dispatch.arguments) or str(irq_dispatch.global_value_type.get_function_return()) != "void":
                raise CompileError("IRQ dispatcher must have signature void irqc_dispatch(void)")
            # Vector slots execute one instruction. R30 is reserved as their
            # indirect branch target; ordinary lowering never uses this register.
            c.imm(11, 0xFFFF0044)
            c.mem("STH", 0, 11)
            c.imm(30, ("label", "__llvm_src32_irq_entry"))
            c.imm(11, 0xFFFF0100)
            c.imm(10, (0x0B << 26) | (30 << 21))  # JR R30
            for line in range(16):
                c.mem("ST", 10, 11, line * 4)
        c.imm(12, ("label", symbol(self.entry)))
        c.emit("JALR R12")
        c.emit("HALT")
        stop = c.fresh("stop")
        c.label(stop)
        c.jump(stop)
        if irq_dispatch is not None:
            c.label("__llvm_src32_irq_entry")
            # Preserve all interrupted registers, with an 8-byte-aligned frame.
            c.add_const(28, 28, -128)
            for register in range(1, 32):
                if register != 28:
                    c.mem("ST", register, 28, (register - 1) * 4)
            c.imm(12, ("label", symbol("irqc_dispatch")))
            c.emit("JALR R12")
            for register in range(1, 32):
                if register != 28:
                    c.mem("LD", register, 28, (register - 1) * 4)
            c.add_const(28, 28, 128)
            c.emit("IRET")
        for function in self.definitions.values():
            linkage = function.linkage.name
            if linkage not in ("external", "internal", "private"):
                raise CompileError(f"function @{function.name}: unsupported linkage {linkage}")
            _Function(self, function).emit()
        functions = {name: c.labels[symbol(name)] for name in self.definitions}
        base = _round(c.pc, 8)
        self.image.assign(base, functions)
        if self.origin < self.stack_top <= self.image.end:
            raise CompileError("initial stack top overlaps the generated code/data image")
        c.resolve(self.image)
        lines = c.lines + self.image.emit(functions, base)
        return "\n".join(lines) + "\n"


def compile_module(module, *, origin=0, stack_top=0x00100000, entry="main"):
    """Lower an official llvmlite ModuleRef into standalone SRC32 assembly.

    The module is verified by LLVM, keeps its big-endian 32-bit target layout,
    and must contain all referenced external definitions. Unsupported semantics
    produce CompileError with the function, block, and instruction context.
    """
    if not 0 <= origin <= 0xFFFFFFFC or origin % 4:
        raise CompileError("origin must be a word-aligned 32-bit address")
    if not 8 <= stack_top <= 0xFFFFFFF8 or stack_top % 8:
        raise CompileError("stack top must be an 8-byte-aligned 32-bit address")
    try:
        module.verify()
    except RuntimeError as exc:
        raise CompileError(f"LLVM verification failed: {exc}") from exc
    try:
        return _Module(module, origin, stack_top, entry).compile()
    except CompileError:
        raise
    except (ValueError, TypeError, KeyError, IndexError, RuntimeError) as exc:
        raise CompileError(f"cannot lower LLVM module: {exc}") from exc
