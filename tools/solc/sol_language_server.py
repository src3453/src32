#!/usr/bin/env python3
"""Minimal stdio LSP server for sol, using the canonical sol compiler."""
from __future__ import annotations
import json, os, re, sys
from pathlib import Path
from urllib.parse import unquote, urlparse
sys.path.insert(0, str(Path(__file__).resolve().parent))
from sol_vm import tokenize, compile_program, SolVMError, parse_number, RawAssemblyToken
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

NAMESPACE_RE = re.compile(r"^\s*!namespace\s+([A-Za-z_][A-Za-z0-9_]*)")
FUNCTION_RE = re.compile(r"(?:^|\s)fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*(\(([^)]*)\))?")
DECLARATION_RE = re.compile(r"^\s*!(const|var|define)\s+([A-Za-z_][A-Za-z0-9_]*)")
LOCAL_RE = re.compile(r"(?:^|\s)local\s+([A-Za-z_][A-Za-z0-9_]*)")
RAW_BLOCK_RE = re.compile(r"^\s*!(asm|data|db)\b")

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

class Server:
    def __init__(self): self.docs = {}
    def path(self, uri):
        p=urlparse(uri)
        return unquote(p.path) if p.scheme == "file" else uri
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
        out=[]; toks=tokenize(text,self.path(uri)); fn_names=set(); params=set(); locals_=set(); consts=set(); vars_=set()
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
            return {"id":ident,"result":{"capabilities":{"textDocumentSync":{"openClose":True,"change":1},"semanticTokensProvider":{"legend":{"tokenTypes":TOKEN_TYPES,"tokenModifiers":TOKEN_MODIFIERS},"full":True},"documentSymbolProvider":True,"diagnosticProvider":{"interFileDependencies":False,"workspaceDiagnostics":False,"identifier":"sol"}}}}
        if method=="textDocument/diagnostic":
            uri=p["textDocument"]["uri"]
            return {"id":ident,"result":{"kind":"full","items":self.analyze(uri,self.docs.get(uri,""))}}
        if method=="shutdown": return {"id":ident,"result":None}
        if method=="exit": return None
        if method in ("textDocument/didOpen","textDocument/didChange"):
            td=p["textDocument"]; text=td.get("text", p.get("contentChanges",[{}])[-1].get("text","")); self.docs[td["uri"]]=text
            return {"method":"textDocument/publishDiagnostics","params":{"uri":td["uri"],"diagnostics":self.analyze(td["uri"],text)}}
        if method=="textDocument/didClose":
            uri=p["textDocument"]["uri"]; self.docs.pop(uri,None); return {"method":"textDocument/publishDiagnostics","params":{"uri":uri,"diagnostics":[]}}
        if method=="textDocument/semanticTokens/full":
            uri=p["textDocument"]["uri"]; return {"id":ident,"result":{"data":sum(self.tokens(uri and self.docs.get(uri,""),uri),[])}}
        if method=="textDocument/documentSymbol":
            uri=p["textDocument"]["uri"]; return {"id":ident,"result":self.symbols(self.docs.get(uri,""))}
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
        try: result=s.handle(json.loads(body))
        except Exception as e: result={"id":None,"error":{"code":-32603,"message":str(e)}}
        if result is not None: send(result)
        if json.loads(body).get("method")=="exit": break
if __name__=="__main__": main()
