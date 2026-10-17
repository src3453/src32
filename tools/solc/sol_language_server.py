#!/usr/bin/env python3
"""Minimal stdio LSP server for sol, using the canonical sol compiler."""
from __future__ import annotations
import json, os, re, sys
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import unquote, urlparse
sys.path.insert(0, str(Path(__file__).resolve().parent))
from sol_vm import tokenize, compile_program, SolVMError, parse_number, RawAssemblyToken, LABEL_RE, QUALIFIED_RE, _decode_string_literal
from sol_compiler import check_static_stack_safety, SolCompileError

TOKEN_TYPES = ["comment", "string", "number", "operator", "directive", "control", "function", "parameter", "local", "variable", "constant", "assignment", "identifier", "namespace"]
TOKEN_MODIFIERS = []
OPS = {"add","sub","mul","div","mod","neg","and","or","xor","shl","shr","eq","neq","lt","gt","le","ge","sgn","not","dup","drop","swap","over","rot","nip","tuck","ld","st","ldb","ldh","stb","sth","stacksize","halt","ret","retn"}
CONTROL = {"fn","if","while","else","end","local"}
DIRECTIVES = {"!include","!define","!undef","!const","!var","!end","!namespace","!required_stack_size","!force_stack_size","!asm","!data","!db","!keepfn","!stack_top","!var_base","!rodata_base"}

# Document symbol kinds (LSP SymbolKind).
KIND_NAMESPACE = 3
KIND_FUNCTION = 12
KIND_VARIABLE = 13
KIND_CONSTANT = 14

# Project scan limits for workspace-wide queries.
IGNORED_DIRS = {"node_modules", "target", "__pycache__", "venv"}
MAX_PROJECT_FILES = 4000
MAX_RESULTS = 2000

NAMESPACE_RE = re.compile(r"^\s*!namespace\s+([A-Za-z_][A-Za-z0-9_]*)")
FUNCTION_RE = re.compile(r"(?:^|\s)fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*(\(([^)]*)\))?")
DECLARATION_RE = re.compile(r"^\s*!(const|var|define)\s+([A-Za-z_][A-Za-z0-9_]*)")
LOCAL_RE = re.compile(r"(?:^|\s)local\s+([A-Za-z_][A-Za-z0-9_]*)")
RAW_BLOCK_RE = re.compile(r"^\s*!(asm|data|db)\b")
WINDOWS_PATH_RE = re.compile(r"^/[A-Za-z]:")

def blank_literals(line):
    """Replace string literals and comments with spaces, keeping the positions."""
    out = []
    in_string = False
    i = 0
    while i < len(line):
        ch = line[i]
        if in_string:
            if ch == "\\" and i + 1 < len(line):
                out.append("  ")
                i += 2
                continue
            out.append(" ")
            if ch == '"':
                in_string = False
            i += 1
            continue
        if ch == '"':
            in_string = True
            out.append(" ")
            i += 1
            continue
        if ch == "#":
            out.append(" " * (len(line) - i))
            break
        out.append(ch)
        i += 1
    return "".join(out)

def span(start_line, start_char, end_line, end_char):
    return {"start": {"line": start_line, "character": start_char}, "end": {"line": end_line, "character": end_char}}

def symbol(name, kind, range_, selection, children=None, detail=None):
    result = {"name": name, "kind": kind, "range": range_, "selectionRange": selection}
    if detail:
        result["detail"] = detail
    if children is not None:
        result["children"] = children
    return result

@dataclass
class IncludeMarker:
    """Namespace scope opened (or closed) by an aliased `!include "f" as name`."""
    alias: str
    entering: bool
    token: object = None

class Decl:
    """A declaration that a reference resolves to."""

    __slots__ = ("name", "kind", "label", "uri", "line", "character", "length", "namespace", "detail")

    def __init__(self, name, kind, label, uri, location, namespace=None, detail=None, length=None):
        self.name = name
        self.kind = kind
        self.label = label
        self.uri = uri
        self.line = location.line - 1
        self.character = location.column - 1
        self.length = len(name) if length is None else length
        self.namespace = namespace
        self.detail = detail

    def range(self):
        return span(self.line, self.character, self.line, self.character + self.length)

    def location(self):
        return {"uri": self.uri, "range": self.range()}

    def key(self):
        return (os.path.normcase(self.uri), self.line, self.character, self.name)

    def signature(self):
        if self.label == "fn":
            return f"fn {self.name} {self.detail or '()'}"
        if self.label in ("argument", "local"):
            return f"{self.label} {self.name} ({self.detail})"
        return f"{self.label} {self.name}"

class Ref:
    """A name occurrence that resolves to a declaration."""

    __slots__ = ("name", "uri", "line", "character", "length", "namespace", "scope", "write")

    def __init__(self, name, uri, location, length, namespace, scope, write=False, offset=0):
        self.name = name
        self.uri = uri
        self.line = location.line - 1
        self.character = location.column - 1 + offset
        self.length = length
        self.namespace = namespace
        self.scope = scope
        self.write = write

    def range(self):
        return span(self.line, self.character, self.line, self.character + self.length)

    def location(self):
        return {"uri": self.uri, "range": self.range()}

class Scope:
    """Function body scope holding the argument and local declarations of one function."""

    __slots__ = ("params", "locals", "start", "end")

    def __init__(self):
        self.params = {}
        self.locals = {}
        self.start = 0
        self.end = 0

class Index:
    """Declarations, references and resolution tables of one include closure.

    `symbols` mirrors the compiler symbol tables: qualified names hit one namespace
    only, unqualified names are stored and looked up per namespace with the global
    scope as the fallback."""

    def __init__(self):
        self.symbols = {}
        self.namespaces = {}
        self.functions = []
        self.decls = {}
        self.refs = {}
        self.decl_positions = set()

    def add_decl(self, decl):
        self.decls.setdefault(decl.uri, []).append(decl)

    def add_symbol(self, key, decl):
        self.symbols.setdefault(key, decl)
        self.add_decl(decl)

    def add_ref(self, ref):
        self.refs.setdefault(ref.uri, []).append(ref)

    def find_ref(self, uri, position):
        line, character = position["line"], position["character"]
        trailing = None
        for ref in self.refs.get(uri, []):
            if ref.line != line:
                continue
            if ref.character <= character < ref.character + ref.length:
                return ref
            if character == ref.character + ref.length:
                trailing = trailing or ref
        return trailing

    def find_decl(self, uri, position):
        line, character = position["line"], position["character"]
        for decl in self.decls.get(uri, []):
            if decl.line == line and decl.character <= character <= decl.character + decl.length:
                return decl
        return None

    def resolve(self, ref):
        name = ref.name
        if "." not in name:
            if ref.scope is not None:
                local = ref.scope.locals.get(name) or ref.scope.params.get(name)
                if local is not None:
                    return local
            return self.symbols.get((ref.namespace, name)) or self.symbols.get((None, name)) or self.namespaces.get(name)
        qualifier, member = name.split(".", 1)
        return self.symbols.get((qualifier, member))

class Server:
    def __init__(self):
        self.docs = {}
        self.doc_paths = {}
        self.roots = []
        self.file_token_cache = {}

    def path(self, uri):
        p=urlparse(uri)
        return unquote(p.path) if p.scheme == "file" else uri

    def is_file(self, uri):
        return urlparse(uri).scheme == "file"

    def fs_path(self, uri):
        """Filesystem path of a file URI; Windows file URIs keep a leading slash."""
        raw = self.path(uri)
        return raw[1:] if WINDOWS_PATH_RE.match(raw) else raw

    def uri_for_path(self, path):
        key = os.path.normcase(os.path.abspath(path))
        return self.doc_paths.get(key) or Path(os.path.abspath(path)).as_uri()

    def remember(self, uri):
        if self.is_file(uri):
            self.doc_paths[os.path.normcase(os.path.abspath(self.fs_path(uri)))] = uri

    def doc_text(self, uri):
        if uri in self.docs:
            return self.docs[uri]
        try:
            return Path(self.fs_path(uri)).read_text(encoding="utf-8")
        except OSError:
            return None

    def file_tokens(self, path):
        """Tokenize a file on disk, cached by modification time and size."""
        try:
            info = os.stat(path)
        except OSError:
            return []
        key = os.path.normcase(os.path.abspath(path))
        cached = self.file_token_cache.get(key)
        if cached is not None and cached[0] == (info.st_mtime_ns, info.st_size):
            return cached[1]
        try:
            tokens = tokenize(Path(path).read_text(encoding="utf-8"), path)
        except (OSError, UnicodeDecodeError, SolVMError):
            tokens = []
        self.file_token_cache[key] = ((info.st_mtime_ns, info.st_size), tokens)
        return tokens

    def source_tokens(self, uri, text):
        if text is None:
            text = self.doc_text(uri)
        if text is not None:
            try:
                return tokenize(text, self.fs_path(uri))
            except SolVMError:
                # In-flight buffers (unterminated string or !asm) cannot be tokenized.
                return []
        return self.file_tokens(self.fs_path(uri))

    def expand(self, uri, text, out, seen, path_map):
        """Flatten a document and its includes, tagging aliased includes like the compiler."""
        path = self.fs_path(uri)
        tokens = self.source_tokens(uri, text)
        i = 0
        while i < len(tokens):
            token = tokens[i]
            if str(token) == "!include":
                filename = tokens[i + 1] if i + 1 < len(tokens) else None
                alias = None
                consumed = 2
                if i + 3 < len(tokens) and str(tokens[i + 2]) == "as":
                    alias = tokens[i + 3]
                    consumed = 4
                target = None
                if filename is not None:
                    name = str(filename)
                    if name.startswith('"') and name.endswith('"'):
                        try:
                            name = _decode_string_literal(name)
                        except SolVMError:
                            name = ""
                    target = os.path.abspath(os.path.join(os.path.dirname(path), name))
                if alias is not None:
                    out.append(IncludeMarker(str(alias), True, alias))
                if target is not None:
                    key = os.path.normcase(target)
                    if key not in seen and os.path.exists(target):
                        seen.add(key)
                        target_uri = self.uri_for_path(target)
                        path_map[key] = target_uri
                        self.expand(target_uri, None, out, seen, path_map)
                if alias is not None:
                    out.append(IncludeMarker(str(alias), False))
                i += consumed
                continue
            out.append(token)
            i += 1

    def index_of(self, uri):
        path_map = {}
        index = Index()
        if not self.is_file(uri):
            return index
        path = self.fs_path(uri)
        key = os.path.normcase(os.path.abspath(path))
        path_map[key] = uri
        seen = {key}
        flat = []
        self.expand(uri, self.docs.get(uri), flat, seen, path_map)
        self.scan(flat, path_map, index)
        return index

    def decl_for(self, token, name, kind, label, path_map, namespace=None, detail=None, length=None):
        location = token.location
        path = location.path or ""
        uri = path_map.get(os.path.normcase(os.path.abspath(path))) or self.uri_for_path(path)
        return Decl(name, kind, label, uri, location, namespace=namespace, detail=detail, length=length)

    def scan(self, flat, path_map, index):
        """Collect declarations (pass 1) and references (pass 2) from an expanded stream."""
        i = 0
        namespace = None
        while i < len(flat):
            token = flat[i]
            if isinstance(token, IncludeMarker):
                if token.entering:
                    namespace = token.alias
                    decl = self.decl_for(token.token, token.alias, KIND_NAMESPACE, "!namespace", path_map)
                    index.namespaces.setdefault(token.alias, decl)
                    index.add_decl(decl)
                else:
                    namespace = None
                i += 1
                continue
            if isinstance(token, RawAssemblyToken):
                i += 1
                continue
            text = str(token)
            if text == "!namespace":
                name_token = flat[i + 1] if i + 1 < len(flat) else None
                if name_token is None or not LABEL_RE.match(str(name_token)):
                    i += 1
                    continue
                namespace = str(name_token)
                decl = self.decl_for(name_token, namespace, KIND_NAMESPACE, "!namespace", path_map)
                index.namespaces.setdefault(namespace, decl)
                index.add_decl(decl)
                index.decl_positions.add(self.token_pos(name_token))
                i += 3 if i + 2 < len(flat) and str(flat[i + 2]) == "-" else 2
                continue
            if text == "!end":
                namespace = None
                i += 1
                continue
            if text in ("!data", "!db"):
                j = i + 1
                while j < len(flat) and str(flat[j]) != "!end":
                    j += 1
                i = j + 1
                continue
            if text == "fn":
                name_token = flat[i + 1] if i + 1 < len(flat) else None
                if name_token is None or not LABEL_RE.match(str(name_token)):
                    i += 1
                    continue
                name = str(name_token)
                j = i + 2
                args = []
                if j < len(flat) and str(flat[j]) == "(":
                    j += 1
                    while j < len(flat) and str(flat[j]) != ")":
                        args.append(flat[j])
                        j += 1
                    j += 1
                if j < len(flat) and str(flat[j]) == ":":
                    j += 1
                body = j
                while j < len(flat) and str(flat[j]) != ";":
                    j += 1
                scope = Scope()
                scope.start, scope.end = i, j
                decl = self.decl_for(name_token, name, KIND_FUNCTION, "fn", path_map, namespace=namespace,
                                     detail="(" + " ".join(str(arg) for arg in args) + ")")
                index.add_symbol((namespace, name), decl)
                index.decl_positions.add(self.token_pos(name_token))
                for arg in args:
                    if not LABEL_RE.match(str(arg)):
                        continue
                    arg_name = str(arg)
                    index.decl_positions.add(self.token_pos(arg))
                    if arg_name not in scope.params:
                        scope.params[arg_name] = self.decl_for(arg, arg_name, KIND_VARIABLE, "argument", path_map,
                                                               namespace=namespace, detail=f"of {name}")
                        index.add_decl(scope.params[arg_name])
                for k in range(body, j):
                    if str(flat[k]) != "local" or k + 1 >= len(flat):
                        continue
                    local_token = flat[k + 1]
                    if not LABEL_RE.match(str(local_token)):
                        continue
                    local_name = str(local_token)
                    index.decl_positions.add(self.token_pos(local_token))
                    if local_name not in scope.locals:
                        scope.locals[local_name] = self.decl_for(local_token, local_name, KIND_VARIABLE, "local", path_map,
                                                                  namespace=namespace, detail=f"of {name}")
                        index.add_decl(scope.locals[local_name])
                index.functions.append(scope)
                i = j + 1
                continue
            if text in ("!const", "!var", "!define"):
                name_token = flat[i + 1] if i + 1 < len(flat) else None
                if name_token is None or not LABEL_RE.match(str(name_token)):
                    i += 1
                    continue
                name = str(name_token)
                index.decl_positions.add(self.token_pos(name_token))
                if text == "!define":
                    index.add_symbol((None, name), self.decl_for(name_token, name, KIND_CONSTANT, "!define", path_map))
                else:
                    kind = KIND_CONSTANT if text == "!const" else KIND_VARIABLE
                    index.add_symbol((namespace, name), self.decl_for(name_token, name, kind, text, path_map, namespace=namespace))
                i += 1
                continue
            i += 1

        spans = sorted(index.functions, key=lambda scope: scope.start)
        position = 0
        scope = None
        i = 0
        while i < len(flat):
            if scope is not None and i > scope.end:
                scope = None
            while position < len(spans) and spans[position].start <= i:
                scope = spans[position]
                position += 1
            token = flat[i]
            if isinstance(token, IncludeMarker):
                namespace = token.alias if token.entering else None
                i += 1
                continue
            if isinstance(token, RawAssemblyToken):
                i += 1
                continue
            text = str(token)
            if text == "!namespace":
                name_token = flat[i + 1] if i + 1 < len(flat) else None
                if name_token is None or not LABEL_RE.match(str(name_token)):
                    i += 1
                    continue
                namespace = str(name_token)
                i += 3 if i + 2 < len(flat) and str(flat[i + 2]) == "-" else 2
                continue
            if text == "!end":
                namespace = None
                i += 1
                continue
            if text in ("!data", "!db"):
                j = i + 1
                while j < len(flat) and str(flat[j]) != "!end":
                    j += 1
                i = j + 1
                continue
            if text == "fn":
                j = i + 2
                if j < len(flat) and str(flat[j]) == "(":
                    j += 1
                    while j < len(flat) and str(flat[j]) != ")":
                        j += 1
                    j += 1
                if j < len(flat) and str(flat[j]) == ":":
                    j += 1
                i = j
                continue
            if not text or text.startswith("!") or text in OPS or text in CONTROL or text in DIRECTIVES:
                i += 1
                continue
            write = text.startswith(">")
            name = text[1:] if write else text
            if not QUALIFIED_RE.match(name) or self.token_pos(token) in index.decl_positions:
                i += 1
                continue
            uri = path_map.get(os.path.normcase(os.path.abspath(token.location.path or ""))) or self.uri_for_path(token.location.path)
            if "." in name:
                qualifier, member = name.split(".", 1)
                base = 1 if write else 0
                index.add_ref(Ref(qualifier, uri, token.location, len(qualifier), namespace, scope, offset=base))
                index.add_ref(Ref(name, uri, token.location, len(member), namespace, scope, write=write, offset=base + len(qualifier) + 1))
            else:
                index.add_ref(Ref(name, uri, token.location, len(name), namespace, scope, write=write, offset=1 if write else 0))
            i += 1
        return index

    def token_pos(self, token):
        location = token.location
        return (os.path.normcase(os.path.abspath(location.path or "")), location.line, location.column)

    def target_at(self, index, uri, position):
        ref = index.find_ref(uri, position)
        if ref is not None:
            decl = index.resolve(ref)
            if decl is not None:
                return decl
        return index.find_decl(uri, position)

    def project_files(self):
        files = {uri for uri in self.docs if self.is_file(uri)}
        for root in self.roots:
            if not self.is_file(root):
                continue
            base = self.fs_path(root)
            for directory, dirnames, filenames in os.walk(base):
                dirnames[:] = [name for name in dirnames if name not in IGNORED_DIRS and not name.startswith(".")]
                for filename in filenames:
                    if filename.endswith(".sol"):
                        files.add(self.uri_for_path(os.path.join(directory, filename)))
                if len(files) >= MAX_PROJECT_FILES:
                    break
        return sorted(files)

    def same_file(self, one, other):
        return os.path.normcase(self.fs_path(one)) == os.path.normcase(self.fs_path(other))

    def find_references(self, root_index, uri, target, include_declaration):
        key = target.key()
        found = {}
        for project_uri in self.project_files():
            index = root_index if project_uri == uri else self.index_of(project_uri)
            for refs in index.refs.values():
                for ref in refs:
                    decl = index.resolve(ref)
                    if decl is not None and decl.key() == key:
                        found[(os.path.normcase(self.fs_path(ref.uri)), ref.line, ref.character)] = ref.location()
            if len(found) >= MAX_RESULTS:
                break
        if include_declaration:
            found[(os.path.normcase(self.fs_path(target.uri)), target.line, target.character)] = target.location()
        return [found[item] for item in sorted(found)]

    def display_path(self, uri):
        path = self.fs_path(uri)
        for root in self.roots:
            if not self.is_file(root):
                continue
            try:
                relative = os.path.relpath(path, self.fs_path(root))
            except ValueError:
                continue
            if not relative.startswith(".."):
                return relative.replace("\\", "/")
        return path.replace("\\", "/")

    def diag(self, message, severity=1, loc=None):
        d={"severity":severity,"message":message,"source":"sol"}
        if loc:
            d["range"]={"start":{"line":loc.line-1,"character":loc.column-1},"end":{"line":loc.line-1,"character":loc.column}}
        else: d["range"]={"start":{"line":0,"character":0},"end":{"line":0,"character":1}}
        return d
    def analyze(self, uri, text):
        diagnostics=[]
        try:
            program=compile_program(text, source_path=self.path(uri), remove_unused_functions=False)
            check_static_stack_safety(program)
        except (SolVMError, SolCompileError) as e:
            msg=str(e); loc=None
            m=re.match(r".*:(\d+):(\d+):\s*(.*)$", msg)
            if m:
                from sol_vm import SourceLocation
                loc=SourceLocation(self.path(uri),int(m.group(1)),int(m.group(2))); msg=m.group(3)
            diagnostics.append(self.diag(msg,1,loc))
        return diagnostics
    def tokens(self, text, uri):
        out=[]
        try: toks=tokenize(text,self.path(uri))
        except SolVMError: return out
        fn_names=set(); params=set(); locals_=set(); consts=set(); vars_=set()
        namespace=None; block_stack=[]; namespace_names=set()
        for i,t in enumerate(toks):
            if t == "!namespace" and i+1 < len(toks):
                namespace=str(toks[i+1]); namespace_names.add(namespace); block_stack.append("namespace")
                continue
            if t == "!include":
                j=i+1
                while j < len(toks) and str(toks[j]).startswith('"'): j+=1
                if j+1 < len(toks) and str(toks[j])=="as": namespace_names.add(str(toks[j+1]))
                continue
            if t in {"!asm","!data","!db"}:
                block_stack.append(str(t))
                continue
            if t == "!end":
                if block_stack and block_stack.pop() == "namespace": namespace=None
                continue
            if t == "fn" and i+1 < len(toks):
                name=str(toks[i+1]); fn_names.add(name)
                if namespace: fn_names.add(namespace+"."+name)
                j=i+2
                while j < len(toks) and toks[j] != ")":
                    if toks[j] != "(": params.add(str(toks[j]))
                    j += 1
            if t=="!const" and i+1<len(toks):
                name=str(toks[i+1]); consts.add(name)
                if namespace: consts.add(namespace+"."+name)
            if t=="!var" and i+1<len(toks):
                name=str(toks[i+1]); vars_.add(name)
                if namespace: vars_.add(namespace+"."+name)
            if t=="local" and i+1<len(toks): locals_.add(str(toks[i+1]))
        for t in toks:
            if isinstance(t, RawAssemblyToken):
                out.append((t.location.line-1,t.location.column-1,4,TOKEN_TYPES.index("directive"),0))
                continue
            s=str(t)
            if s in {";","(",")"}: continue
            typ=None
            if "." in s and not s.startswith('"') and not s.startswith(">") and (s[0].isalpha() or s[0]=="_"):
                prefix,member=s.split(".",1)
                member_type="function" if s in fn_names else "constant" if s in consts else "variable" if s in vars_ else "identifier"
                out.append((t.location.line-1,t.location.column-1,len(prefix),TOKEN_TYPES.index("namespace"),0))
                out.append((t.location.line-1,t.location.column+len(prefix),len(member),TOKEN_TYPES.index(member_type),0))
                continue
            if s.startswith('"'): typ="string"
            elif s in OPS: typ="operator"
            elif s in DIRECTIVES: typ="directive"
            elif s in CONTROL: typ="control"
            else:
                try: parse_number(s); typ="number"
                except Exception:
                    if s in fn_names: typ="function"
                    elif s in params: typ="parameter"
                    elif s in locals_: typ="local"
                    elif s in consts: typ="constant"
                    elif s in vars_: typ="variable"
                    elif s in namespace_names: typ="namespace"
                    elif s.startswith(">"): typ="assignment"
                    else: typ="identifier"
            if typ: out.append((t.location.line-1,t.location.column-1,len(s),TOKEN_TYPES.index(typ),0))
        out.sort(); result=[]; pl=pc=0
        for line,col,length,kind,mod in out:
            result.append([line-pl, col-(pc if line==pl else 0), length, kind, mod]); pl,pc=line,col
        return result
    def skip_raw_block(self, lines, index):
        index += 1
        while index < len(lines) and lines[index].strip() != "!end": index += 1
        return index + 1
    def statement_end(self, lines, start, column):
        """Return (line, character) just past the ';' terminating the statement starting at (start, column)."""
        index = start
        while index < len(lines):
            if RAW_BLOCK_RE.match(lines[index]):
                index = self.skip_raw_block(lines, index)
                continue
            found = blank_literals(lines[index]).find(";", column if index == start else 0)
            if found >= 0: return index, found + 1
            index += 1
        last = len(lines) - 1
        return last, (len(lines[last]) if lines else 0)
    def symbols(self, text):
        lines=text.splitlines(); root=[]; stack=[]
        def add(sym): (stack[-1]["children"] if stack else root).append(sym)
        index=0
        while index < len(lines):
            raw_line=lines[index]
            if RAW_BLOCK_RE.match(raw_line):
                index=self.skip_raw_block(lines,index); continue
            line=blank_literals(raw_line)
            namespace=NAMESPACE_RE.match(line)
            if namespace:
                sym=symbol(namespace.group(1),KIND_NAMESPACE,span(index,0,index,len(raw_line)),span(index,namespace.start(1),index,namespace.end(1)),children=[])
                add(sym); stack.append(sym); index+=1; continue
            if line.strip()=="!end":
                if stack: stack.pop()["range"]["end"]={"line":index,"character":len(raw_line)}
                index+=1; continue
            function=FUNCTION_RE.search(line)
            if function:
                name=function.group(1); params=function.group(3) or ""
                end_line,end_char=self.statement_end(lines,index,function.end())
                children=[]; body=index
                while body <= end_line:
                    if RAW_BLOCK_RE.match(lines[body]):
                        body=self.skip_raw_block(lines,body); continue
                    for local in LOCAL_RE.finditer(blank_literals(lines[body])):
                        children.append(symbol(local.group(1),KIND_VARIABLE,span(body,local.start(1),body,local.end(1)),span(body,local.start(1),body,local.end(1))))
                    body+=1
                sym=symbol(name,KIND_FUNCTION,span(index,function.start(),end_line,end_char),span(index,function.start(1),index,function.end(1)),
                           children=children,detail="("+" ".join(params.split())+")")
                add(sym); index=end_line+1; continue
            declaration=DECLARATION_RE.match(line)
            if declaration:
                keyword,name=declaration.group(1),declaration.group(2)
                add(symbol(name,KIND_CONSTANT if keyword in ("const","define") else KIND_VARIABLE,
                           span(index,0,index,len(raw_line)),span(index,declaration.start(2),index,declaration.end(2))))
                index+=1; continue
            index+=1
        if lines:
            for open_symbol in stack:
                open_symbol["range"]["end"]={"line":len(lines)-1,"character":len(lines[-1])}
        return root
    def handle(self, req):
        method=req.get("method"); p=req.get("params") or {}; ident=req.get("id")
        if method=="initialize":
            roots=[]
            for folder in p.get("workspaceFolders") or []:
                if folder.get("uri"): roots.append(folder["uri"])
            if not roots:
                root=p.get("rootUri") or p.get("rootPath")
                if root:
                    roots.append(root if "://" in root else Path(root).resolve().as_uri())
            self.roots=roots
            return {"id":ident,"result":{"capabilities":{"textDocumentSync":{"openClose":True,"change":1},"semanticTokensProvider":{"legend":{"tokenTypes":TOKEN_TYPES,"tokenModifiers":TOKEN_MODIFIERS},"full":True},"documentSymbolProvider":True,"definitionProvider":True,"declarationProvider":True,"referencesProvider":True,"hoverProvider":True,"documentHighlightProvider":True,"workspaceSymbolProvider":True,"diagnosticProvider":{"interFileDependencies":False,"workspaceDiagnostics":False,"identifier":"sol"}}}}
        if method=="textDocument/diagnostic":
            uri=p["textDocument"]["uri"]
            return {"id":ident,"result":{"kind":"full","items":self.analyze(uri,self.docs.get(uri,""))}}
        if method=="shutdown": return {"id":ident,"result":None}
        if method=="exit": return None
        if method in ("textDocument/didOpen","textDocument/didChange"):
            td=p["textDocument"]; text=td.get("text", p.get("contentChanges",[{}])[-1].get("text","")); self.docs[td["uri"]]=text
            self.remember(td["uri"])
            return {"method":"textDocument/publishDiagnostics","params":{"uri":td["uri"],"diagnostics":self.analyze(td["uri"],text)}}
        if method=="textDocument/didClose":
            uri=p["textDocument"]["uri"]; self.docs.pop(uri,None); return {"method":"textDocument/publishDiagnostics","params":{"uri":uri,"diagnostics":[]}}
        if method=="textDocument/semanticTokens/full":
            uri=p["textDocument"]["uri"]; return {"id":ident,"result":{"data":sum(self.tokens(uri and self.docs.get(uri,""),uri),[])}}
        if method=="textDocument/documentSymbol":
            uri=p["textDocument"]["uri"]; return {"id":ident,"result":self.symbols(self.docs.get(uri,""))}
        if method in ("textDocument/definition","textDocument/declaration"):
            uri=p["textDocument"]["uri"]; index=self.index_of(uri)
            decl=self.target_at(index, uri, p["position"])
            return {"id":ident,"result":[decl.location()] if decl else None}
        if method=="textDocument/hover":
            uri=p["textDocument"]["uri"]; index=self.index_of(uri)
            decl=self.target_at(index, uri, p["position"])
            if decl is None: return {"id":ident,"result":None}
            value=f"```sol\n{decl.signature()}\n```\n\n`{self.display_path(decl.uri)}:{decl.line+1}:{decl.character+1}`"
            return {"id":ident,"result":{"contents":{"kind":"markdown","value":value}}}
        if method=="textDocument/documentHighlight":
            uri=p["textDocument"]["uri"]; index=self.index_of(uri)
            decl=self.target_at(index, uri, p["position"])
            if decl is None: return {"id":ident,"result":None}
            highlights=[]
            for ref in index.refs.get(uri, []):
                if index.resolve(ref) is decl:
                    highlights.append({"range":ref.range(),"kind":3 if ref.write else 2})
            if self.same_file(decl.uri, uri) and not any(item["range"] == decl.range() for item in highlights):
                highlights.append({"range":decl.range(),"kind":1})
            highlights.sort(key=lambda item:(item["range"]["start"]["line"],item["range"]["start"]["character"]))
            return {"id":ident,"result":highlights}
        if method=="textDocument/references":
            uri=p["textDocument"]["uri"]; index=self.index_of(uri)
            decl=self.target_at(index, uri, p["position"])
            if decl is None: return {"id":ident,"result":[]}
            include=p.get("context",{}).get("includeDeclaration",True)
            return {"id":ident,"result":self.find_references(index, uri, decl, include)}
        if method=="workspace/symbol":
            query=(p.get("query") or "").lower()
            found={}
            for project_uri in self.project_files():
                index=self.index_of(project_uri)
                for decl in index.decls.get(project_uri, []):
                    if decl.label in ("argument", "local") or query not in decl.name.lower(): continue
                    key=decl.key()
                    if key not in found:
                        found[key]={"name":decl.name,"kind":decl.kind,"location":decl.location(),"containerName":decl.namespace or ""}
                if len(found) >= MAX_RESULTS: break
            return {"id":ident,"result":[found[key] for key in sorted(found)]}
        if ident is not None: return {"id":ident,"error":{"code":-32601,"message":"Method not found"}}
        return None

def send(msg):
    raw=json.dumps(msg,separators=(",",":")).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(raw)}\r\n\r\n".encode() + raw)
    sys.stdout.buffer.flush()
def main():
    s=Server()
    while True:
        line=sys.stdin.buffer.readline()
        if not line: break
        if not line.lower().startswith(b"content-length:"): continue
        n=int(line.split(b":",1)[1]); sys.stdin.buffer.readline(); body=sys.stdin.buffer.read(n)
        request=None
        try:
            request=json.loads(body)
            result=s.handle(request)
        except Exception as e:
            # Answer with the request id, otherwise the client waits forever.
            result={"id":(request or {}).get("id"),"error":{"code":-32603,"message":str(e)}}
        if result is not None: send(result)
        if (request or {}).get("method")=="exit": break
if __name__=="__main__": main()
