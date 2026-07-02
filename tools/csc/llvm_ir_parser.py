"""
llvm_ir_parser.py
Parse a subset of LLVM IR (textual .ll) and produce a csc CodeEmitter
that can be fed into backend_src32.emit_src32().

Supported LLVM IR subset:
  - define i32 @name(i32, ...) { ... }
  - ret i32 <value>
  - <result> = add i32 <a>, <b>
  - <result> = sub i32 <a>, <b>
  - <result> = mul i32 <a>, <b>
  - <result> = sdiv i32 <a>, <b>
  - <result> = icmp <cond> i32 <a>, <b>   (cond: eq, ne, slt, sgt, sle, sge)
  - <result> = alloca i32
  - store i32 <value>, i32* <ptr>
  - <result> = load i32, i32* <ptr>
  - <result> = call i32 @name(args...)
  - br label %dest
  - br i1 <cond>, label %true, label %false
  - <result> = phi i32 [ %val1, %label1 ], [ %val2, %label2 ]
  - Labels: %label:

Values:
  - integer constants
  - %named registers / temporaries

Limitations:
  - Only i32 type is supported.
  - No global variables, arrays, structs, or pointers beyond alloca.
  - No vector/SIMD, no floating point.
  - Functions are assumed to return i32.
  - No exception handling, no inline asm.
"""
import re
from collections import OrderedDict
from csc_gen import CodeEmitter


# ---------------------------------------------------------------------------
# Tokenizer for LLVM IR
# ---------------------------------------------------------------------------

TOKEN_RE = re.compile(r"""
    ;[^\n]*                  # line comment
    | @"[^"]*"               # string constant (named)
    | "[^"]*"                # string constant
    | [-]?[0-9]+             # integer literal
    | [%@][a-zA-Z0-9_.]+     # identifier (%tmp, @func)
    | i[0-9]+\*?             # type like i32, i1, i32*
    | [a-zA-Z_][a-zA-Z0-9_]* # keyword / bare identifier
    | [(){}\[\]<>=!,]        # punctuation
    | \S                     # any other non-space (catch-all)
""", re.VERBOSE)


def tokenize_ll(source: str):
    """Yield a flat list of string tokens from LLVM IR source."""
    tokens = []
    pos = 0
    text = source
    while pos < len(text):
        # skip whitespace
        m = re.match(r'\s+', text[pos:])
        if m:
            pos += m.end()
            continue
        m = TOKEN_RE.match(text, pos)
        if not m:
            raise SyntaxError(f"Unexpected character {text[pos]!r} at position {pos}")
        tok = m.group(0)
        # skip line comments
        if tok.startswith(';'):
            pos = m.end()
            continue
        tokens.append(tok)
        pos = m.end()
    return tokens


# ---------------------------------------------------------------------------
# AST: basic block / function / module
# ---------------------------------------------------------------------------

class LLVMFunction:
    """Represents one LLVM function with its basic blocks."""
    def __init__(self, name, params):
        self.name = name
        self.params = params  # list of (type, name)
        self.blocks = OrderedDict()  # label -> list of instructions
        self.block_order = []  # ordered list of labels

    def add_block(self, label):
        if label not in self.blocks:
            self.blocks[label] = []
            self.block_order.append(label)

    def add_instr(self, label, instr):
        self.add_block(label)
        self.blocks[label].append(instr)


class LLVMModule:
    """A parsed LLVM module: a collection of functions."""
    def __init__(self):
        self.functions = OrderedDict()  # name -> LLVMFunction


# ---------------------------------------------------------------------------
# Parser
# ---------------------------------------------------------------------------

def _is_int(tok):
    """Check if a token is an integer literal."""
    if tok is None:
        return False
    if tok.startswith('-'):
        return tok[1:].isdigit()
    return tok.isdigit()


def _parse_value(tokens, pos):
    """Parse a value: integer constant or %identifier. Returns (value, new_pos)."""
    if pos[0] >= len(tokens):
        raise SyntaxError("Unexpected end of input, expected value")
    t = tokens[pos[0]]
    if _is_int(t):
        pos[0] += 1
        return int(t)
    if t.startswith('%') or t.startswith('@'):
        pos[0] += 1
        return t
    raise SyntaxError(f"Expected value, got {t!r}")


def parse_llvm_ir(source: str) -> LLVMModule:
    """Parse LLVM IR source into an LLVMModule."""
    tokens = tokenize_ll(source)
    pos = [0]
    module = LLVMModule()

    while pos[0] < len(tokens):
        tok = tokens[pos[0]]
        if tok == 'define':
            func = _parse_define(tokens, pos)
            module.functions[func.name] = func
        elif tok == 'declare':
            _skip_declare(tokens, pos)
        else:
            raise SyntaxError(f"Unexpected top-level token {tok!r}")

    return module


def _skip_declare(tokens, pos):
    """Skip a declare statement. Declare format: declare <ret_type> @name(<params>)"""
    # consume 'declare'
    pos[0] += 1
    # consume everything until we hit 'define', 'declare', or '}'
    while pos[0] < len(tokens):
        t = tokens[pos[0]]
        if t in ('define', 'declare', '}'):
            return
        pos[0] += 1


def _parse_define(tokens, pos):
    """Parse a define i32 @name(params) { body }."""
    if tokens[pos[0]] != 'define':
        raise SyntaxError(f"Expected 'define', got {tokens[pos[0]]!r}")
    pos[0] += 1

    if pos[0] >= len(tokens):
        raise SyntaxError("Unexpected end after 'define'")
    ret_type = tokens[pos[0]]
    pos[0] += 1

    if pos[0] >= len(tokens) or not tokens[pos[0]].startswith('@'):
        raise SyntaxError(f"Expected function name starting with @, got {tokens[pos[0]]!r}")
    fname = tokens[pos[0]][1:]  # strip @
    pos[0] += 1

    if pos[0] >= len(tokens) or tokens[pos[0]] != '(':
        raise SyntaxError(f"Expected '(', got {tokens[pos[0]]!r}")
    pos[0] += 1  # consume '('

    params = []
    while tokens[pos[0]] != ')':
        ptype = tokens[pos[0]]
        pos[0] += 1
        pname = tokens[pos[0]]
        pos[0] += 1
        params.append((ptype, pname))
        if tokens[pos[0]] == ',':
            pos[0] += 1

    pos[0] += 1  # consume ')'

    if pos[0] >= len(tokens) or tokens[pos[0]] != '{':
        raise SyntaxError(f"Expected '{{', got {tokens[pos[0]]!r}")
    pos[0] += 1

    func = LLVMFunction(fname, params)
    current_label = None

    while tokens[pos[0]] != '}':
        t = tokens[pos[0]]

        # Label: either "name:" (single token) or "name" followed by ":"
        if t.endswith(':'):
            current_label = t[:-1]
            func.add_block(current_label)
            pos[0] += 1
            continue
        elif t != '}' and pos[0] + 1 < len(tokens) and tokens[pos[0] + 1] == ':':
            current_label = t
            func.add_block(current_label)
            pos[0] += 2
            continue

        if current_label is None:
            current_label = 'entry'
            func.add_block(current_label)

        instr = _parse_instruction(tokens, pos)
        func.add_instr(current_label, instr)

    pos[0] += 1  # consume '}'
    return func


def _parse_instruction(tokens, pos):
    """Parse a single LLVM instruction and return a dict."""
    t = tokens[pos[0]]

    # terminators
    if t == 'ret':
        return _parse_ret(tokens, pos)
    if t == 'br':
        return _parse_br(tokens, pos)
    if t == 'store':
        return _parse_store(tokens, pos)
    if t == 'unreachable':
        pos[0] += 1
        return {'op': 'unreachable'}

    # instructions that produce a value: <result> = ...
    if t.startswith('%') and pos[0] + 1 < len(tokens) and tokens[pos[0] + 1] == '=':
        result = t
        pos[0] += 2  # skip %name and '='
        op = tokens[pos[0]]
        pos[0] += 1

        if op in ('add', 'sub', 'mul', 'sdiv', 'and', 'or', 'shl', 'lshr', 'ashr'):
            return _parse_binop(tokens, pos, result, op)
        elif op == 'icmp':
            return _parse_icmp(tokens, pos, result)
        elif op == 'load':
            return _parse_load(tokens, pos, result)
        elif op == 'alloca':
            return _parse_alloca(tokens, pos, result)
        elif op == 'call':
            return _parse_call(tokens, pos, result)
        elif op == 'phi':
            return _parse_phi(tokens, pos, result)
        elif op in ('zext', 'sext', 'trunc'):
            # Simplify: treat as identity for i32
            pos[0] += 1  # source type
            val = _parse_value(tokens, pos)
            if pos[0] < len(tokens) and tokens[pos[0]] == 'to':
                pos[0] += 1
                pos[0] += 1  # dest type
            return {'op': 'assign', 'result': result, 'value': val}
        else:
            raise SyntaxError(f"Unsupported instruction {op!r}")

    raise SyntaxError(f"Unexpected instruction starting with {t!r}")


def _parse_ret(tokens, pos):
    """Parse: ret i32 <value>"""
    pos[0] += 1  # consume 'ret'
    ty = tokens[pos[0]]
    pos[0] += 1
    val = _parse_value(tokens, pos)
    return {'op': 'ret', 'type': ty, 'value': val}


def _parse_binop(tokens, pos, result, op):
    """Parse: <result> = <op> i32 <a>, <b>"""
    ty = tokens[pos[0]]
    pos[0] += 1
    a = _parse_value(tokens, pos)
    if tokens[pos[0]] != ',':
        raise SyntaxError(f"Expected ',', got {tokens[pos[0]]!r}")
    pos[0] += 1
    b = _parse_value(tokens, pos)
    return {'op': op, 'result': result, 'type': ty, 'a': a, 'b': b}


def _parse_icmp(tokens, pos, result):
    """Parse: <result> = icmp <cond> i32 <a>, <b>"""
    cond = tokens[pos[0]]
    pos[0] += 1
    ty = tokens[pos[0]]
    pos[0] += 1
    a = _parse_value(tokens, pos)
    if tokens[pos[0]] != ',':
        raise SyntaxError(f"Expected ',', got {tokens[pos[0]]!r}")
    pos[0] += 1
    b = _parse_value(tokens, pos)
    return {'op': 'icmp', 'result': result, 'cond': cond, 'type': ty, 'a': a, 'b': b}


def _parse_load(tokens, pos, result):
    """Parse: <result> = load i32, i32* <ptr>. 'load' already consumed."""
    ty = tokens[pos[0]]
    pos[0] += 1
    if tokens[pos[0]] != ',':
        raise SyntaxError(f"Expected ',' in load, got {tokens[pos[0]]!r}")
    pos[0] += 1
    ptr_ty = tokens[pos[0]]
    pos[0] += 1
    ptr = _parse_value(tokens, pos)
    return {'op': 'load', 'result': result, 'type': ty, 'ptr': ptr}


def _parse_alloca(tokens, pos, result):
    """Parse: <result> = alloca i32. 'alloca' already consumed."""
    ty = tokens[pos[0]]
    pos[0] += 1
    if pos[0] < len(tokens) and tokens[pos[0]] == ',':
        pos[0] += 1  # ','
        pos[0] += 1  # type
        pos[0] += 1  # count
    return {'op': 'alloca', 'result': result, 'type': ty}


def _parse_store(tokens, pos):
    """Parse: store i32 <value>, i32* <ptr>"""
    pos[0] += 1  # consume 'store'
    ty = tokens[pos[0]]
    pos[0] += 1
    val = _parse_value(tokens, pos)
    if tokens[pos[0]] != ',':
        raise SyntaxError(f"Expected ',' in store, got {tokens[pos[0]]!r}")
    pos[0] += 1
    ptr_ty = tokens[pos[0]]
    pos[0] += 1
    ptr = _parse_value(tokens, pos)
    return {'op': 'store', 'type': ty, 'value': val, 'ptr': ptr}


def _parse_call(tokens, pos, result):
    """Parse: <result> = call i32 @name(args...). 'call' already consumed."""
    ty = tokens[pos[0]]
    pos[0] += 1
    name = tokens[pos[0]]
    if not name.startswith('@'):
        raise SyntaxError(f"Expected function name, got {name!r}")
    fname = name[1:]
    pos[0] += 1
    if tokens[pos[0]] != '(':
        raise SyntaxError(f"Expected '(', got {tokens[pos[0]]!r}")
    pos[0] += 1

    args = []
    while tokens[pos[0]] != ')':
        arg_ty = tokens[pos[0]]
        pos[0] += 1
        arg_val = _parse_value(tokens, pos)
        args.append((arg_ty, arg_val))
        if tokens[pos[0]] == ',':
            pos[0] += 1

    pos[0] += 1  # consume ')'
    return {'op': 'call', 'result': result, 'type': ty, 'name': fname, 'args': args}


def _parse_phi(tokens, pos, result):
    """Parse: <result> = phi i32 [ %val, %label ], .... 'phi' already consumed."""
    ty = tokens[pos[0]]
    pos[0] += 1
    entries = []
    while True:
        if tokens[pos[0]] != '[':
            raise SyntaxError(f"Expected '[', got {tokens[pos[0]]!r}")
        pos[0] += 1
        val = _parse_value(tokens, pos)
        if tokens[pos[0]] != ',':
            raise SyntaxError(f"Expected ',', got {tokens[pos[0]]!r}")
        pos[0] += 1
        label = _parse_value(tokens, pos)
        if tokens[pos[0]] != ']':
            raise SyntaxError(f"Expected ']', got {tokens[pos[0]]!r}")
        pos[0] += 1
        entries.append((val, label))
        if pos[0] < len(tokens) and tokens[pos[0]] == ',':
            pos[0] += 1
        else:
            break
    return {'op': 'phi', 'result': result, 'type': ty, 'entries': entries}


def _strip_percent(name):
    """Strip leading % from a label name if present."""
    if isinstance(name, str) and name.startswith('%'):
        return name[1:]
    return name


def _parse_br(tokens, pos):
    """Parse: br label %dest  OR  br i1 <cond>, label %true, label %false"""
    pos[0] += 1  # consume 'br'
    t = tokens[pos[0]]
    if t == 'label':
        pos[0] += 1
        dest = _parse_value(tokens, pos)
        return {'op': 'br', 'dest': _strip_percent(dest)}
    else:
        cond_ty = tokens[pos[0]]
        pos[0] += 1
        cond = _parse_value(tokens, pos)
        if tokens[pos[0]] != ',':
            raise SyntaxError(f"Expected ',', got {tokens[pos[0]]!r}")
        pos[0] += 1
        if tokens[pos[0]] != 'label':
            raise SyntaxError(f"Expected 'label', got {tokens[pos[0]]!r}")
        pos[0] += 1
        true_dest = _parse_value(tokens, pos)
        if tokens[pos[0]] != ',':
            raise SyntaxError(f"Expected ',', got {tokens[pos[0]]!r}")
        pos[0] += 1
        if tokens[pos[0]] != 'label':
            raise SyntaxError(f"Expected 'label', got {tokens[pos[0]]!r}")
        pos[0] += 1
        false_dest = _parse_value(tokens, pos)
        return {'op': 'br_cond', 'cond': cond,
                'true': _strip_percent(true_dest),
                'false': _strip_percent(false_dest)}


# ---------------------------------------------------------------------------
# Code generation: convert LLVM module to csc CodeEmitter
# ---------------------------------------------------------------------------

LLVM_COND_MAP = {
    'eq': '==',
    'ne': '!=',
    'slt': '<',
    'sgt': '>',
    'sle': '<=',
    'sge': '>=',
}


def llvm_module_to_emitter(module: LLVMModule) -> CodeEmitter:
    """Convert a parsed LLVM module to a csc CodeEmitter."""
    emitter = CodeEmitter()
    func_labels = {}  # function name -> bytecode index
    max_reg_count = 0  # track max locals across all functions

    for fname, func in module.functions.items():
        func_reg_map = {}
        func_label_map = {}
        fixups = []

        def freg(name, _fr=func_reg_map):
            if name not in _fr:
                _fr[name] = len(_fr)
            return _fr[name]

        def femit_value(val):
            if isinstance(val, int):
                emitter.emit('PUSH_CONST', val)
            elif isinstance(val, str) and val.startswith('%'):
                idx = freg(val)
                emitter.emit('LOAD_VAR', idx)
            else:
                raise SyntaxError(f"Cannot emit value {val!r}")

        def femit_store(dest, src_val):
            if isinstance(dest, str) and dest.startswith('%'):
                idx = freg(dest)
                femit_value(src_val)
                emitter.emit('STORE_VAR', idx)
            else:
                raise SyntaxError(f"Cannot store to {dest!r}")

        # Record function entry point
        func_labels[fname] = len(emitter.code)

        # Save return address
        emitter.emit('SAVE_RET')

        # Bind parameters to local registers
        for _, pname in func.params:
            idx = freg(pname)
            emitter.emit('STORE_VAR', idx)

        # Emit basic blocks
        for block_label in func.block_order:
            func_label_map[block_label] = len(emitter.code)
            for instr in func.blocks[block_label]:
                op = instr['op']

                if op == 'ret':
                    femit_value(instr['value'])
                    emitter.emit('RETURN')

                elif op in ('add', 'sub', 'mul', 'sdiv', 'and', 'or'):
                    femit_value(instr['a'])
                    femit_value(instr['b'])
                    if op == 'add':
                        emitter.emit('ADD')
                    elif op == 'sub':
                        emitter.emit('SUB')
                    elif op == 'mul':
                        emitter.emit('MUL')
                    elif op == 'sdiv':
                        emitter.emit('DIV')
                    elif op == 'and':
                        emitter.emit('AND')
                    elif op == 'or':
                        emitter.emit('OR')
                    idx = freg(instr['result'])
                    emitter.emit('STORE_VAR', idx)

                elif op == 'icmp':
                    femit_value(instr['a'])
                    femit_value(instr['b'])
                    cond = instr['cond']
                    csc_op = LLVM_COND_MAP.get(cond)
                    if csc_op is None:
                        raise SyntaxError(f"Unsupported icmp condition {cond!r}")
                    emitter.emit('CMP', csc_op)
                    idx = freg(instr['result'])
                    emitter.emit('STORE_VAR', idx)

                elif op == 'alloca':
                    freg(instr['result'])

                elif op == 'load':
                    ptr = instr['ptr']
                    femit_value(ptr)
                    idx = freg(instr['result'])
                    emitter.emit('STORE_VAR', idx)

                elif op == 'store':
                    ptr = instr['ptr']
                    val = instr['value']
                    femit_store(ptr, val)

                elif op == 'call':
                    for _, arg_val in reversed(instr['args']):
                        femit_value(arg_val)
                    callee = instr['name']
                    emitter.emit('CALL', callee)
                    idx = freg(instr['result'])
                    emitter.emit('STORE_VAR', idx)

                elif op == 'phi':
                    if instr['entries']:
                        val, _ = instr['entries'][0]
                        femit_store(instr['result'], val)

                elif op == 'br':
                    dest = instr['dest']
                    emitter.emit('JUMP', 'PLACEHOLDER')
                    fixups.append((len(emitter.code) - 1, dest))

                elif op == 'br_cond':
                    femit_value(instr['cond'])
                    emitter.emit('JUMP_IF_FALSE', 'PLACEHOLDER')
                    fixups.append((len(emitter.code) - 1, instr['false']))
                    emitter.emit('JUMP', 'PLACEHOLDER')
                    fixups.append((len(emitter.code) - 1, instr['true']))

                elif op == 'assign':
                    femit_store(instr['result'], instr['value'])

                elif op == 'unreachable':
                    pass  # no-op

                else:
                    raise SyntaxError(f"Unsupported LLVM op {op!r}")

        # Track max register count across functions
        max_reg_count = max(max_reg_count, len(func_reg_map))

        # Fix up jumps for this function
        for bc_idx, target_label in fixups:
            if target_label in func_label_map:
                emitter.code[bc_idx] = (emitter.code[bc_idx][0], func_label_map[target_label])
            else:
                raise SyntaxError(f"Unknown label {target_label!r} in function {fname}")

    # Attach function labels to emitter for backend
    emitter.func_labels = func_labels
    # Attach var_count so backend can allocate variable space
    emitter.var_count = max(max_reg_count, 1)  # at least 1 for safety
    return emitter


def compile_llvm_ir(source: str) -> CodeEmitter:
    """Parse LLVM IR source and return a CodeEmitter."""
    module = parse_llvm_ir(source)
    return llvm_module_to_emitter(module)
