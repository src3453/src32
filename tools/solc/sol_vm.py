"""sol VM (HLE) for development-time validation."""

from __future__ import annotations

import ast
from dataclasses import dataclass
import os
import re

LABEL_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
NUMBER_RE = re.compile(r"^-?[0-9]+u?$|^0[xX][0-9a-fA-F]+u?$|^0[bB][01]+u?$")
INT32_MIN = -(2**31)
INT32_MAX = 2**31 - 1
UINT32_MAX = 2**32 - 1

STRING_POOL_BASE = 0x00020000
STACK_SIZE_BYTES = 0x00100000


class SolVMError(RuntimeError):
    pass


@dataclass(frozen=True)
class Instruction:
    op: str
    arg: int | str | None = None


@dataclass(frozen=True)
class Program:
    instructions: list[Instruction]
    labels: dict[str, int]
    functions: dict[str, int]
    read_only_data: list[tuple[int, bytes]]


def to_i32(value: int) -> int:
    value &= UINT32_MAX
    if value > INT32_MAX:
        return value - (2**32)
    return value


def tokenize(source: str) -> list[str]:
    tokens: list[str] = []
    current: list[str] = []
    in_string = False
    escaped = False
    i = 0
    while i < len(source):
        ch = source[i]
        if in_string:
            current.append(ch)
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                tokens.append("".join(current))
                current = []
                in_string = False
            elif ch == "\n":
                raise SolVMError("unterminated string literal")
            i += 1
            continue

        if ch == "#":
            if current:
                tokens.append("".join(current))
                current = []
            while i < len(source) and source[i] != "\n":
                i += 1
            continue

        if ch.isspace():
            if current:
                tokens.append("".join(current))
                current = []
            i += 1
            continue

        if ch in {";", "(", ")"}:
            if current:
                tokens.append("".join(current))
                current = []
            tokens.append(ch)
            i += 1
            continue

        if ch == '"':
            if current:
                tokens.append("".join(current))
                current = []
            current.append(ch)
            in_string = True
            escaped = False
            i += 1
            continue

        current.append(ch)
        i += 1

    if in_string:
        raise SolVMError("unterminated string literal")
    if current:
        tokens.append("".join(current))
    return tokens


def _decode_string_literal(token: str) -> str:
    try:
        value = ast.literal_eval(token)
    except (SyntaxError, ValueError) as exc:
        raise SolVMError(f"invalid string literal: {token}") from exc
    if not isinstance(value, str):
        raise SolVMError(f"invalid string literal: {token}")
    return value


def parse_number(token: str) -> int:
    is_unsigned = token.endswith("u")
    core = token[:-1] if is_unsigned else token
    if core == "":
        raise SolVMError("invalid numeric literal: empty")

    try:
        is_hex = core.startswith(("0x", "0X"))
        is_bin = core.startswith(("0b", "0B"))
        if is_hex:
            value = int(core, 16)
        elif is_bin:
            value = int(core, 2)
        else:
            value = int(core, 10)
    except ValueError as exc:
        raise SolVMError(f"invalid numeric literal: {token}") from exc

    if is_hex or is_bin:
        if value < 0 or value > UINT32_MAX:
            raise SolVMError(f"hex/binary literal out of range: {token}")
    elif is_unsigned:
        if value < 0 or value > UINT32_MAX:
            raise SolVMError(f"unsigned literal out of range: {token}")
    elif value < INT32_MIN or value > INT32_MAX:
        raise SolVMError(f"signed literal out of range: {token}")
    return to_i32(value)

STRING_POOL_BASE = 0x00020000
STACK_SIZE_BYTES = 0x00100000

def compile_program(source: str, var_base: int = 0x00100000, read_only_data_base: int = STRING_POOL_BASE, source_path: str | None = None, included_paths: set | None = None) -> Program:
    included_paths = set() if included_paths is None else set(included_paths)
    base_dir = os.path.dirname(os.path.abspath(source_path)) if source_path else None

    def expand_includes(tokens: list[str], current_base: str | None, seen: set[str]) -> list[str]:
        out: list[str] = []
        i = 0
        while i < len(tokens):
            tok = tokens[i]
            if tok == "!include":
                if i + 1 >= len(tokens):
                    raise SolVMError("!include requires a filename")
                fname_tok = tokens[i + 1]
                if fname_tok.startswith('"') and fname_tok.endswith('"') and len(fname_tok) >= 2:
                    fname = _decode_string_literal(fname_tok)
                else:
                    fname = fname_tok
                if current_base:
                    include_path = os.path.abspath(os.path.join(current_base, fname))
                else:
                    include_path = os.path.abspath(fname)
                if include_path in seen:
                    raise SolVMError(f"circular include detected: {include_path}")
                if not os.path.exists(include_path):
                    raise SolVMError(f"include file not found: {include_path}")
                seen.add(include_path)
                included_text = open(include_path, "r", encoding="utf-8").read()
                expanded = expand_includes(tokenize(included_text), os.path.dirname(include_path), seen)
                out.extend(expanded)
                i += 2
                continue
            out.append(tok)
            i += 1
        return out

    tokens = expand_includes(tokenize(source), base_dir, included_paths)
    SOLC_DEBUG = os.getenv("SOLC_DEBUG") == "1"
    if SOLC_DEBUG:
        print("DEBUG: tokens after include expansion ->", tokens)

    instructions: list[Instruction] = []
    labels: dict[str, int] = {}
    read_only_data: list[tuple[int, bytes]] = []

    constants: dict[str, int] = {}
    variables: dict[str, int] = {}
    macros: dict[str, list[str]] = {}
    next_var_addr = var_base
    functions: dict[str, dict] = {}

    string_literals: dict[str, int] = {}
    next_string_addr = read_only_data_base
    control_label_counter = 0

    def intern_string_literal(token: str) -> int:
        nonlocal next_string_addr
        text = _decode_string_literal(token)
        if text in string_literals:
            return string_literals[text]
        data = text.encode("utf-8") + b"\x00"
        addr = next_string_addr
        next_string_addr += len(data)
        string_literals[text] = addr
        read_only_data.append((addr, data))
        return addr

    def new_hidden_label(kind: str) -> str:
        nonlocal control_label_counter
        label = f"__solc_{kind}_{control_label_counter}"
        control_label_counter += 1
        return label

    def expand_token_recursive(tok: str, seen: set[str]) -> list[str]:
        if tok not in macros:
            return [tok]
        if tok in seen:
            raise SolVMError(f"circular macro detected: {tok}")
        seen.add(tok)
        out: list[str] = []
        for part in macros[tok]:
            if part in macros:
                out.extend(expand_token_recursive(part, seen))
            else:
                out.append(part)
        seen.remove(tok)
        return out

    simple_ops = {
        "add",
        "sub",
        "mul",
        "div",
        "mod",
        "neg",
        "and",
        "or",
        "xor",
        "not",
        "shl",
        "shr",
        "dup",
        "drop",
        "swap",
        "over",
        "rot",
        "nip",
        "tuck",
        "ld",
        "st",
        "ldb",
        "ldh",
        "stb",
        "sth",
        "eq",
        "neq",
        "lt",
        "gt",
        "le",
        "ge",
        "sgn",
        "ret",
        "retn",
        "stacksize",
        "halt",
    }

    def store_target(name: str, locals_list: list[tuple[str, int | None]] | None) -> bool:
        if locals_list is not None:
            for local_index, (local_name, _) in enumerate(locals_list):
                if local_name == name:
                    instructions.append(Instruction("local_addr", local_index))
                    instructions.append(Instruction("st"))
                    return True
        if name in variables:
            instructions.append(Instruction("push", variables[name]))
            instructions.append(Instruction("st"))
            return True
        return False

    def compile_word_stream(words: list[str], *, current_func: str | None = None, locals_list: list[tuple[str, int | None]] | None = None, start_index: int = 0, stop_tokens: set[str] | None = None) -> int:
        nonlocal next_string_addr, next_var_addr
        terminal_ops = {"ret", "retn", "halt"}
        i = start_index
        while i < len(words):
            tok = words[i]

            if stop_tokens is not None and tok in stop_tokens:
                return i

            if tok in macros:
                words[i:i + 1] = expand_token_recursive(tok, set())
                continue

            if tok == ";":
                i += 1
                continue

            if tok == "if":
                false_label = new_hidden_label("if_false")
                instructions.append(Instruction("jnz", false_label))
                i = compile_word_stream(words, current_func=current_func, locals_list=locals_list, start_index=i + 1, stop_tokens={"else", "end"})
                if i >= len(words):
                    raise SolVMError("if requires terminating end")
                then_terminates = bool(instructions) and instructions[-1].op in terminal_ops
                if words[i] == "else":
                    end_label = None
                    if not then_terminates:
                        end_label = new_hidden_label("if_end")
                        instructions.append(Instruction("jmp", end_label))
                    labels[false_label] = len(instructions)
                    i = compile_word_stream(words, current_func=current_func, locals_list=locals_list, start_index=i + 1, stop_tokens={"end"})
                    if i >= len(words) or words[i] != "end":
                        raise SolVMError("else requires terminating end")
                    if end_label is not None:
                        labels[end_label] = len(instructions)
                    i += 1
                    continue
                if words[i] != "end":
                    raise SolVMError(f"unexpected token in if block: {words[i]}")
                labels[false_label] = len(instructions)
                i += 1
                continue

            if tok == "while":
                start_label = new_hidden_label("while_start")
                labels[start_label] = len(instructions)
                i = compile_word_stream(words, current_func=current_func, locals_list=locals_list, start_index=i + 1, stop_tokens={"end"})
                if i >= len(words) or words[i] != "end":
                    raise SolVMError("while requires terminating end")
                instructions.append(Instruction("jz", start_label))
                i += 1
                continue

            if tok in {"else", "end"}:
                if stop_tokens is not None and tok in stop_tokens:
                    return i
                raise SolVMError(f"unexpected token: {tok}")

            if tok.startswith("@"): 
                raise SolVMError("labels are hidden; use if/else/while/end")

            if tok in {"jmp", "jz", "jnz"}:
                raise SolVMError(f"raw jumps are hidden; use if/else/while/end: {tok}")

            if tok.startswith("\\!"):
                tok = tok[1:]

            if tok.startswith("!"):
                if current_func is not None:
                    raise SolVMError(f"directives inside function not supported: {tok}")
                if tok == "!const":
                    if i + 2 >= len(words):
                        raise SolVMError("!const requires a name and a value")
                    name = words[i + 1]
                    if not LABEL_RE.match(name):
                        raise SolVMError(f"invalid constant name: {name}")
                    val_tok = words[i + 2]
                    if not NUMBER_RE.match(val_tok):
                        raise SolVMError(f"invalid constant value: {val_tok}")
                    value = parse_number(val_tok)
                    if name in constants:
                        raise SolVMError(f"duplicate constant: {name}")
                    constants[name] = value
                    i += 3
                    continue

                if tok == "!var":
                    if i + 1 >= len(words):
                        raise SolVMError("!var requires a name and optional initial value")
                    name = words[i + 1]
                    if not LABEL_RE.match(name):
                        raise SolVMError(f"invalid variable name: {name}")
                    init_value = 0
                    consumed = 2
                    if i + 2 < len(words) and NUMBER_RE.match(words[i + 2]):
                        init_value = parse_number(words[i + 2])
                        consumed = 3
                    if name in variables:
                        raise SolVMError(f"duplicate variable: {name}")
                    addr = next_var_addr
                    next_var_addr += 4
                    variables[name] = addr
                    instructions.append(Instruction("push", init_value))
                    instructions.append(Instruction("push", addr))
                    instructions.append(Instruction("st"))
                    i += consumed
                    continue

                if tok == "!define":
                    if i + 2 >= len(words):
                        raise SolVMError("!define requires a name and a value")
                    name = words[i + 1]
                    if not LABEL_RE.match(name):
                        raise SolVMError(f"invalid macro name: {name}")
                    value_tok = words[i + 2]
                    if name in macros:
                        raise SolVMError(f"duplicate macro: {name}")
                    macros[name] = [value_tok]
                    i += 3
                    continue

                if tok in {"!data", "!db"}:
                    is_db = tok == "!db"
                    j = i + 1
                    data_tokens: list[str] = []
                    while j < len(words) and words[j] != "!end":
                        data_tokens.append(words[j])
                        j += 1
                    if j >= len(words) or words[j] != "!end":
                        raise SolVMError(f"{tok} requires terminating !end")
                    arr = bytearray()
                    if not is_db:
                        next_string_addr = (next_string_addr + 3) & ~3
                    for dt in data_tokens:
                        if not NUMBER_RE.match(dt):
                            raise SolVMError(f"invalid data token for {tok}: {dt}")
                        val = parse_number(dt)
                        if is_db:
                            arr.append(val & 0xFF)
                        else:
                            u32 = val & 0xFFFFFFFF
                            arr.extend([(u32 >> 24) & 0xFF, (u32 >> 16) & 0xFF, (u32 >> 8) & 0xFF, u32 & 0xFF])
                    read_only_data.append((next_string_addr, bytes(arr)))
                    next_string_addr += len(arr)
                    i = j + 1
                    continue

                raise SolVMError(f"unsupported directive: {tok}")

            if tok.startswith(">"):
                name = tok[1:]
                if not LABEL_RE.match(name):
                    raise SolVMError(f"invalid variable name for store: {name}")
                if not store_target(name, locals_list):
                    raise SolVMError(f"undefined variable: {name}")
                i += 1
                continue

            if tok.startswith('"') and tok.endswith('"'):
                instructions.append(Instruction("push", intern_string_literal(tok)))
                i += 1
                continue

            if tok in simple_ops:
                instructions.append(Instruction(tok))
                i += 1
                continue

            if tok in functions:
                instructions.append(Instruction("call", tok))
                i += 1
                continue

            if tok in constants:
                instructions.append(Instruction("push", constants[tok]))
                i += 1
                continue

            if LABEL_RE.match(tok) and tok in variables:
                instructions.append(Instruction("push", variables[tok]))
                instructions.append(Instruction("ld"))
                i += 1
                continue

            if tok.startswith("__ARG_"):
                if current_func is None:
                    raise SolVMError("argument marker found outside function body")
                idx_str = tok.split("__ARG_")[-1]
                try:
                    arg_index = int(idx_str)
                except ValueError as exc:
                    raise SolVMError(f"invalid ARG marker in function {current_func}: {tok}") from exc
                instructions.append(Instruction("arg", arg_index))
                i += 1
                continue

            if tok.startswith("__LOCAL_"):
                if current_func is None:
                    raise SolVMError("local marker found outside function body")
                idx_str = tok.split("__LOCAL_")[-1]
                try:
                    local_index = int(idx_str)
                except ValueError as exc:
                    raise SolVMError(f"invalid LOCAL marker in function {current_func}: {tok}") from exc
                instructions.append(Instruction("local_addr", local_index))
                instructions.append(Instruction("ld"))
                i += 1
                continue

            if not NUMBER_RE.match(tok):
                raise SolVMError(f"unknown word: {tok}")
            instructions.append(Instruction("push", parse_number(tok)))
            i += 1

        return i

    i = 0
    cleaned_tokens: list[str] = []
    if SOLC_DEBUG:
        print("DEBUG: entering first-pass function collection; tokens length=", len(tokens))
    while i < len(tokens):
        tok = tokens[i]
        if isinstance(tok, str) and tok.startswith("\\!"):
            tok = tok[1:]
        if SOLC_DEBUG and i % 50 == 0:
            print(f"DEBUG first-pass i={i} tok={tok}")
        if tok == "fn":
            if i + 1 >= len(tokens):
                raise SolVMError("fn requires a name")
            name = tokens[i + 1]
            if not LABEL_RE.match(name):
                raise SolVMError(f"invalid function name: {name}")
            j = i + 2
            args: list[str] = []
            if j < len(tokens) and tokens[j] == "(":
                j += 1
                while j < len(tokens) and tokens[j] != ")":
                    args.append(tokens[j])
                    j += 1
                if j >= len(tokens) or tokens[j] != ")":
                    raise SolVMError("unterminated function argument list")
                j += 1
            elif j < len(tokens) and tokens[j].startswith("(") and tokens[j].endswith(")"):
                inner = tokens[j][1:-1].strip()
                args = inner.split() if inner else []
                j += 1
            else:
                raise SolVMError("fn requires argument list in parentheses")
            if j >= len(tokens) or tokens[j] != ":":
                raise SolVMError("fn requires ':' after argument list")
            j += 1
            body_tokens: list[str] = []
            while j < len(tokens) and tokens[j] != ";":
                body_tokens.append(tokens[j])
                j += 1
            if j >= len(tokens) or tokens[j] != ";":
                raise SolVMError("function body not terminated with ';'")

            locals_list: list[tuple[str, int | None]] = []
            k = 0
            while k < len(body_tokens):
                if body_tokens[k] == "local":
                    if k + 1 >= len(body_tokens):
                        raise SolVMError("local requires a name")
                    local_name = body_tokens[k + 1]
                    if not LABEL_RE.match(local_name):
                        raise SolVMError(f"invalid local name: {local_name}")
                    init_value = None
                    consumed = 2
                    if k + 2 < len(body_tokens) and NUMBER_RE.match(body_tokens[k + 2]):
                        init_value = parse_number(body_tokens[k + 2])
                        consumed = 3
                    local_index = len(locals_list)
                    locals_list.append((local_name, init_value))
                    marker = f"__LOCAL_{local_index}"
                    for idx in range(len(body_tokens)):
                        if body_tokens[idx] == local_name:
                            body_tokens[idx] = marker
                    del body_tokens[k:k + consumed]
                    continue
                k += 1

            for idx_arg, arg in enumerate(args):
                marker = f"__ARG_{idx_arg}"
                for idx in range(len(body_tokens)):
                    if body_tokens[idx] == arg:
                        body_tokens[idx] = marker

            if name in functions:
                raise SolVMError(f"duplicate function: {name}")
            functions[name] = {"args": args, "body": body_tokens, "locals": locals_list}
            i = j + 1
            continue

        cleaned_tokens.append(tok)
        i += 1

    compile_word_stream(cleaned_tokens)

    if functions:
        instructions.append(Instruction("halt"))

    for fname, fdata in functions.items():
        if fname in labels:
            raise SolVMError(f"label/function name conflict: {fname}")
        labels[fname] = len(instructions)
        body_tokens = fdata["body"]
        locals_list = fdata.get("locals", [])
        func_code_start = len(instructions)

        for local_index, (_, init_value) in enumerate(locals_list):
            if init_value is not None:
                instructions.append(Instruction("push", init_value))
                instructions.append(Instruction("local_addr", local_index))
                instructions.append(Instruction("st"))

        compile_word_stream(body_tokens, current_func=fname, locals_list=locals_list)

        if len(instructions) == func_code_start or instructions[-1].op not in {"ret", "retn"}:
            instructions.append(Instruction("retn"))

    for inst in instructions:
        if inst.op in {"jmp", "jz", "jnz"} and isinstance(inst.arg, str):
            if inst.arg not in labels:
                raise SolVMError(f"undefined label: {inst.arg}")

    functions_map = {name: {"argcount": len(data["args"]), "n_locals": len(data.get("locals", []))} for name, data in functions.items()}

    return Program(instructions=instructions, labels=labels, functions=functions_map, read_only_data=read_only_data)

'''
            instructions.append(Instruction("push", value))
            i += 1

        return i

    # First pass: collect function definitions and remove them from token stream
    i = 0
    cleaned_tokens: list[str] = []
    if SOLC_DEBUG:
        print('DEBUG: entering first-pass function collection; tokens length=', len(tokens))
    while i < len(tokens):
        tok = tokens[i]
        # normalize escaped bang tokens (shells may escape '!' into '\!')
        if isinstance(tok, str) and tok.startswith("\\!"):
            tok = tok[1:]
        if SOLC_DEBUG and i % 50 == 0:
            print(f'DEBUG first-pass i={i} tok={tok}')
        if tok == "fn":
            # parse: fn name (arg1 arg2 ...) : <body tokens...> ;
            if i + 1 >= len(tokens):
                raise SolVMError("fn requires a name")
            name = tokens[i + 1]
            if not LABEL_RE.match(name):
                raise SolVMError(f"invalid function name: {name}")
            # parse args
            j = i + 2
            args: list[str] = []
            if j < len(tokens) and tokens[j] == "(":
                # standard tokenized form: '(' arg1 arg2 ')' 
                j += 1
                while j < len(tokens) and tokens[j] != ")":
                    args.append(tokens[j])
                    j += 1
                if j >= len(tokens) or tokens[j] != ")":
                    raise SolVMError("unterminated function argument list")
                j += 1
            elif j < len(tokens) and tokens[j].startswith("(") and tokens[j].endswith(")"):
                # compact form: '(a b)'
                inner = tokens[j][1:-1].strip()
                if inner:
                    args = inner.split()
                else:
                    args = []
                j += 1
            else:
                raise SolVMError("fn requires argument list in parentheses")
            if j >= len(tokens) or tokens[j] != ":":
                raise SolVMError("fn requires ':' after argument list")
            j += 1
            # collect body until ';'
            body_tokens: list[str] = []
            while j < len(tokens) and tokens[j] != ";":
                body_tokens.append(tokens[j])
                j += 1
            if j >= len(tokens) or tokens[j] != ";":
                raise SolVMError("function body not terminated with ';'")
            # process local declarations inside body: collect locals and optional inits
            locals_list: list[tuple[str, int | None]] = []
            k = 0
            while k < len(body_tokens):
                if body_tokens[k] == "local":
                    if k + 1 >= len(body_tokens):
                        raise SolVMError("local requires a name")
                    local_name = body_tokens[k + 1]
                    if not LABEL_RE.match(local_name):
                        raise SolVMError(f"invalid local name: {local_name}")
                    # optional init value
                    init_value = None
                    consumed = 2
                    if k + 2 < len(body_tokens) and NUMBER_RE.match(body_tokens[k + 2]):
                        init_value = parse_number(body_tokens[k + 2])
                        consumed = 3
                    local_index = len(locals_list)
                    locals_list.append((local_name, init_value))
                    # replace occurrences of the local name in body with a marker
                    marker = f"__LOCAL_{local_index}"
                    for idx in range(len(body_tokens)):
                        if body_tokens[idx] == local_name:
                            body_tokens[idx] = marker
                    # remove the local declaration tokens
                    del body_tokens[k:k+consumed]
                    continue
                k += 1
            # replace occurrences of arg names with a special ARG marker so the body parser
            # can compile them into runtime 'arg' instructions
            for idx_arg, arg in enumerate(args):
                marker = f"__ARG_{idx_arg}"
                for idx in range(len(body_tokens)):
                    if body_tokens[idx] == arg:
                        body_tokens[idx] = marker
            # store function metadata (args, body tokens, locals)
            if name in functions:
                raise SolVMError(f"duplicate function: {name}")
            functions[name] = {"args": args, "body": body_tokens, "locals": locals_list}
            i = j + 1
            continue
        # not a function definition; keep token
        cleaned_tokens.append(tok)
        i += 1

    # No inlining: keep cleaned tokens as main program tokens
    tokens = cleaned_tokens
    compile_word_stream(tokens)

    # terminate the main program so execution does not fall through into function bodies
    if functions:
        instructions.append(Instruction("halt"))

    # append function bodies after main program so they are not executed unless called
    for fname, fdata in functions.items():
        # mark label for function entry
        if fname in labels:
            raise SolVMError(f"label/function name conflict: {fname}")
        labels[fname] = len(instructions)
        body_tokens = fdata["body"]
        locals_list = fdata.get("locals", [])
        argcount = len(fdata["args"])
        n_locals = len(locals_list)
        # record function code start index so we can append an implicit retn if needed
        func_code_start = len(instructions)
        # At function entry, optional local initializers must run (frame is allocated by caller)
        # Emit initialization sequences for locals with initial values: push init; push local_addr; st
        for local_index, (_, init_value) in enumerate(locals_list):
            if init_value is not None:
                # push init_value, push address of local, st
                compile_word_stream(body_tokens, current_func=fname, locals_list=locals_list)
            instructions.append(Instruction("push", value))
            j += 1
            continue

        # if function body did not contain an explicit ret or retn, append an implicit retn
        if len(instructions) == func_code_start or instructions[-1].op not in {"ret", "retn"}:
            instructions.append(Instruction("retn"))

    # validate jump targets
    for inst in instructions:
        if inst.op in {"jmp", "jz", "jnz"} and isinstance(inst.arg, str):
            if inst.arg not in labels:
                raise SolVMError(f"undefined label: {inst.arg}")

    # build functions mapping name -> metadata for VM/emitter
    functions_map = {name: {"argcount": len(data["args"]), "n_locals": len(data.get("locals", []))} for name, data in functions.items()}

    return Program(instructions=instructions, labels=labels, functions=functions_map, read_only_data=read_only_data)

'''


class SolVM:
    def __init__(self, *, max_steps: int = 1_000_000):
        self.max_steps = max_steps
        self.stack: list[int] = []
        self.memory: dict[int, int] = {}
        self.pc = 0
        self.halted = False
        self.program: Optional[Program] = None
        # call stack for function frames. Each frame is dict with 'ret_pc' and 'args' list
        self.call_stack: list[dict] = []
        self.trace_enabled = False
        self.trace: list[str] = []

    def reset(self) -> None:
        self.stack = []
        self.memory = {}
        self.pc = 0
        self.halted = False
        self.program = None
        # runtime stack pointer (R28) initial default; matches compiler/emitter default
        self.r28 = 0x000FFFFC
        self.call_stack = []
        self.trace = []

    def set_trace(self, enabled: bool) -> None:
        self.trace_enabled = enabled
        self.trace = []

    def clear_trace(self) -> None:
        self.trace = []

    def _record_trace(self, inst: Instruction) -> None:
        if not self.trace_enabled:
            return
        stack_snapshot = list(self.stack)
        arg_text = "" if inst.arg is None else f" arg={inst.arg}"
        self.trace.append(
            f"pc={self.pc} op={inst.op}{arg_text} stack={stack_snapshot} frames={len(self.call_stack)} r28=0x{self.r28:08X}"
        )

    def load(self, source: str, source_path: str | None = None, read_only_data_base: int = STRING_POOL_BASE) -> None:
        self.program = compile_program(source, read_only_data_base=read_only_data_base, source_path=source_path)
        self.pc = 0
        self.halted = False
        # reset runtime stack pointer per program (keep same default)
        self.r28 = 0x000FFFFC
        self.call_stack = []
        self.memory = {}
        self.trace = []
        for addr, data in self.program.read_only_data:
            for offset, byte in enumerate(data):
                self.memory[addr + offset] = byte

    def _pop(self) -> int:
        if not self.stack:
            raise SolVMError("stack underflow")
        return self.stack.pop()

    def _read_mem(self, address: int, size: int) -> int:
        value = 0
        for shift in range(0, size * 8, 8):
            byte = self.memory.get(address + (shift // 8), 0)
            value = (value << 8) | byte
        return to_i32(value)

    def _write_mem(self, address: int, value: int, size: int) -> None:
        value &= 0xFFFFFFFF
        for offset in range(size):
            byte_shift = 8 * (size - 1 - offset)
            byte = (value >> byte_shift) & 0xFF
            self.memory[address + offset] = byte

    def run(self) -> list[int]:
        if self.program is None:
            raise SolVMError("no program loaded")
        program = self.program
        steps = 0
        while self.pc < len(program.instructions) and not self.halted:
            steps += 1
            if steps > self.max_steps:
                raise SolVMError("execution exceeded step limit")
            inst = program.instructions[self.pc]
            self.execute_instruction(inst, program.labels)
        return self.stack

    def run_source(self, source: str, source_path: str | None = None, read_only_data_base: int = STRING_POOL_BASE) -> list[int]:
        self.load(source, source_path=source_path, read_only_data_base=read_only_data_base)
        return self.run()

    def execute_instruction(self, inst: Instruction, labels: dict[str, int]) -> None:
        self._record_trace(inst)
        op = inst.op

        if op == "push":
            assert isinstance(inst.arg, int)
            self.stack.append(inst.arg)
            self.pc += 1
            return

        if op == "arg":
            # push function argument by index from current call frame memory
            assert isinstance(inst.arg, int)
            if not self.call_stack:
                raise SolVMError("'arg' used outside of function frame")
            frame = self.call_stack[-1]
            func_name = frame.get("func_name")
            if func_name is None:
                raise SolVMError("internal error: frame has no func_name for arg access")
            func_meta = self.program.functions.get(func_name)
            if func_meta is None:
                raise SolVMError(f"unknown function metadata for {func_name}")
            argcount = func_meta["argcount"]
            n_locals = func_meta["n_locals"]
            idx = inst.arg
            if idx < 0 or idx >= argcount:
                raise SolVMError(f"argument index out of range: {idx}")
            addr = frame["frame_base"] + 4 * (n_locals + idx)
            self.stack.append(self._read_mem(addr, 4))
            self.pc += 1
            return

        if op == "local_addr":
            # push address of local variable (frame-relative)
            assert isinstance(inst.arg, int)
            if not self.call_stack:
                raise SolVMError("local used outside of function frame")
            frame = self.call_stack[-1]
            addr = frame["frame_base"] + 4 * inst.arg
            self.stack.append(addr)
            self.pc += 1
            return

        if op == "call":
            # start a function call: allocate frame in memory, copy args from caller stack into frame, push frame and jump
            assert isinstance(inst.arg, str)
            func_name = inst.arg
            if self.program is None:
                raise SolVMError("no program loaded")
            if func_name not in self.program.functions:
                raise SolVMError(f"unknown function: {func_name}")
            func_meta = self.program.functions[func_name]
            argcount = func_meta["argcount"]
            n_locals = func_meta["n_locals"]
            frame_size = 4 * (n_locals + argcount)
            # pop arguments from data stack
            args = [0] * argcount
            for j in range(argcount - 1, -1, -1):
                args[j] = self._pop()
            # allocate frame by moving R28
            new_r28 = (self.r28 - frame_size) & 0xFFFFFFFF
            # write args into frame memory
            for j in range(argcount):
                addr = new_r28 + 4 * (n_locals + j)
                self._write_mem(addr, args[j], 4)
            # create frame record (record current data stack height so retn can restore it)
            frame = {"ret_pc": self.pc + 1, "frame_base": new_r28, "func_name": func_name, "stack_height": len(self.stack)}
            # update R28
            self.r28 = new_r28
            self.call_stack.append(frame)
            # jump to function entry
            if func_name not in labels:
                raise SolVMError(f"undefined function label: {func_name}")
            self.pc = labels[func_name]
            return

        if op == "ret":
            # return from function: pop frame and restore R28 and pc (preserve data stack as-is)
            if not self.call_stack:
                raise SolVMError("ret without call frame")
            frame = self.call_stack.pop()
            # restore R28 (frame_base + frame_size)
            func_meta = self.program.functions.get(frame.get("func_name") if frame.get("func_name") else "")
            if func_meta is None:
                # fallback: no meta, keep R28 unchanged
                pass
            else:
                n_locals = func_meta["n_locals"]
                argcount = func_meta["argcount"]
                frame_size = 4 * (n_locals + argcount)
                self.r28 = (frame["frame_base"] + frame_size) & 0xFFFFFFFF
            stack_height = frame["stack_height"]
            if len(self.stack) > stack_height:
                self.stack = self.stack[:stack_height] + [self.stack[-1]]
            else:
                self.stack = self.stack[:stack_height]
            self.pc = frame["ret_pc"]
            return

        if op == "retn":
            # return (no return value): pop frame, restore R28 and pc, and trim any data pushed by callee
            if not self.call_stack:
                raise SolVMError("retn without call frame")
            frame = self.call_stack.pop()
            func_meta = self.program.functions.get(frame.get("func_name") if frame.get("func_name") else "")
            if func_meta is None:
                pass
            else:
                n_locals = func_meta["n_locals"]
                argcount = func_meta["argcount"]
                frame_size = 4 * (n_locals + argcount)
                self.r28 = (frame["frame_base"] + frame_size) & 0xFFFFFFFF
            stack_height = frame["stack_height"]
            if len(self.stack) > stack_height:
                self.stack = self.stack[:stack_height] + [self.stack[-1]]
            else:
                self.stack = self.stack[:stack_height]
            self.pc = frame["ret_pc"]
            return

        if op == "add":
            b = self._pop()
            a = self._pop()
            self.stack.append(to_i32(a + b))
            self.pc += 1
            return

        if op == "sub":
            b = self._pop()
            a = self._pop()
            self.stack.append(to_i32(a - b))
            self.pc += 1
            return

        if op == "mul":
            b = self._pop()
            a = self._pop()
            self.stack.append(to_i32(a * b))
            self.pc += 1
            return

        if op == "and":
            b = self._pop()
            a = self._pop()
            self.stack.append(to_i32(a & b))
            self.pc += 1
            return

        if op == "or":
            b = self._pop()
            a = self._pop()
            self.stack.append(to_i32(a | b))
            self.pc += 1
            return

        if op == "xor":
            b = self._pop()
            a = self._pop()
            self.stack.append(to_i32(a ^ b))
            self.pc += 1
            return

        if op == "not":
            a = self._pop()
            self.stack.append(to_i32(~a))
            self.pc += 1
            return

        if op == "div":
            b = self._pop()
            a = self._pop()
            if b == 0:
                raise SolVMError("division by zero")
            self.stack.append(to_i32(int(a / b)))
            self.pc += 1
            return

        if op == "mod":
            b = self._pop()
            a = self._pop()
            if b == 0:
                raise SolVMError("division by zero")
            self.stack.append(to_i32(a % b))
            self.pc += 1
            return

        if op == "neg":
            a = self._pop()
            self.stack.append(to_i32(-a))
            self.pc += 1
            return

        if op == "shl":
            b = self._pop()
            a = self._pop()
            sh = b & 0x1F
            self.stack.append(to_i32((a & UINT32_MAX) << sh))
            self.pc += 1
            return

        if op == "shr":
            b = self._pop()
            a = self._pop()
            sh = b & 0x1F
            self.stack.append(to_i32(a >> sh))
            self.pc += 1
            return

        if op == "dup":
            if not self.stack:
                raise SolVMError("stack underflow")
            self.stack.append(self.stack[-1])
            self.pc += 1
            return

        if op == "drop":
            self._pop()
            self.pc += 1
            return

        if op == "swap":
            if len(self.stack) < 2:
                raise SolVMError("stack underflow")
            self.stack[-1], self.stack[-2] = self.stack[-2], self.stack[-1]
            self.pc += 1
            return

        if op == "over":
            if len(self.stack) < 2:
                raise SolVMError("stack underflow")
            self.stack.append(self.stack[-2])
            self.pc += 1
            return

        if op == "rot":
            if len(self.stack) < 3:
                raise SolVMError("stack underflow")
            self.stack[-3], self.stack[-2], self.stack[-1] = self.stack[-2], self.stack[-1], self.stack[-3]
            self.pc += 1
            return

        if op == "nip":
            if len(self.stack) < 2:
                raise SolVMError("stack underflow")
            del self.stack[-2]
            self.pc += 1
            return

        if op == "tuck":
            if len(self.stack) < 2:
                raise SolVMError("stack underflow")
            b = self._pop()
            a = self._pop()
            self.stack.extend([b, a, b])
            self.pc += 1
            return

        if op == "ld":
            address = self._pop()
            self.stack.append(self._read_mem(address, 4))
            self.pc += 1
            return

        if op == "st":
            address = self._pop()
            value = self._pop()
            self._write_mem(address, value, 4)
            self.pc += 1
            return

        if op == "ldb":
            address = self._pop()
            self.stack.append(self._read_mem(address, 1))
            self.pc += 1
            return

        if op == "ldh":
            address = self._pop()
            self.stack.append(self._read_mem(address, 2))
            self.pc += 1
            return

        if op == "stb":
            address = self._pop()
            value = self._pop()
            self._write_mem(address, value, 1)
            self.pc += 1
            return

        if op == "sth":
            address = self._pop()
            value = self._pop()
            self._write_mem(address, value, 2)
            self.pc += 1
            return

        if op == "eq":
            b = self._pop()
            a = self._pop()
            self.stack.append(0 if a == b else 1)
            self.pc += 1
            return

        if op == "neq":
            b = self._pop()
            a = self._pop()
            self.stack.append(0 if a != b else 1)
            self.pc += 1
            return

        if op == "lt":
            b = self._pop()
            a = self._pop()
            self.stack.append(0 if a < b else 1)
            self.pc += 1
            return

        if op == "gt":
            b = self._pop()
            a = self._pop()
            self.stack.append(0 if a > b else 1)
            self.pc += 1
            return

        if op == "le":
            b = self._pop()
            a = self._pop()
            self.stack.append(0 if a <= b else 1)
            self.pc += 1
            return

        if op == "ge":
            b = self._pop()
            a = self._pop()
            self.stack.append(0 if a >= b else 1)
            self.pc += 1
            return

        if op == "sgn":
            a = self._pop()
            self.stack.append((to_i32(a) & UINT32_MAX) >> 31)
            self.pc += 1
            return

        if op == "jmp":
            assert isinstance(inst.arg, str)
            self.pc = labels[inst.arg]
            return

        if op == "jz":
            assert isinstance(inst.arg, str)
            cond = self._pop()
            self.pc = labels[inst.arg] if cond == 0 else self.pc + 1
            return

        if op == "jnz":
            assert isinstance(inst.arg, str)
            cond = self._pop()
            self.pc = labels[inst.arg] if cond != 0 else self.pc + 1
            return

        if op == "halt":
            self.halted = True
            self.pc += 1
            return

        if op == "stacksize":
            self.stack.append(STACK_SIZE_BYTES)
            self.pc += 1
            return

        raise SolVMError(f"unknown opcode: {op}")
