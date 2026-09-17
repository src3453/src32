#!/usr/bin/env python3
"""Minimal stdio LSP server for sol, using the canonical sol compiler."""
from __future__ import annotations
import json, os, re, sys
from pathlib import Path
from urllib.parse import unquote, urlparse
sys.path.insert(0, str(Path(__file__).resolve().parent))
from sol_vm import tokenize, compile_program, SolVMError, parse_number
from sol_compiler import check_static_stack_safety, SolCompileError

TOKEN_TYPES = ["comment", "string", "number", "operator", "directive", "control", "function", "parameter", "local", "variable", "constant", "assignment", "identifier"]
TOKEN_MODIFIERS = []
OPS = {"add","sub","mul","div","mod","neg","and","or","xor","shl","shr","eq","neq","lt","gt","le","ge","sgn","not","dup","drop","swap","over","rot","nip","tuck","ld","st","ldb","ldh","stb","sth","stacksize","halt","ret","retn"}
CONTROL = {"fn","if","while","else","end","local"}
DIRECTIVES = {"!include","!define","!undef","!const","!var","!end","!required_stack_size","!force_stack_size","!asm","!data","!db","!keepfn"}

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
        for i,t in enumerate(toks):
            if t == "fn" and i+1 < len(toks):
                fn_names.add(str(toks[i+1])); j=i+2
                while j < len(toks) and toks[j] != ")":
                    if toks[j] != "(": params.add(str(toks[j]))
                    j += 1
            if t=="!const" and i+1<len(toks): consts.add(str(toks[i+1]))
            if t=="!var" and i+1<len(toks): vars_.add(str(toks[i+1]))
            if t=="local" and i+1<len(toks): locals_.add(str(toks[i+1]))
        for t in toks:
            s=str(t); typ=None
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
                    elif s.startswith(">"): typ="assignment"
                    else: typ="identifier"
            if typ: out.append((t.location.line-1,t.location.column-1,len(s),TOKEN_TYPES.index(typ),0))
        out.sort(); result=[]; pl=pc=0
        for line,col,length,kind,mod in out:
            result.append([line-pl, col-(pc if line==pl else 0), length, kind, mod]); pl,pc=line,col
        return result
    def handle(self, req):
        method=req.get("method"); p=req.get("params") or {}; ident=req.get("id")
        if method=="initialize":
            return {"id":ident,"result":{"capabilities":{"textDocumentSync":{"openClose":True,"change":1},"semanticTokensProvider":{"legend":{"tokenTypes":TOKEN_TYPES,"tokenModifiers":TOKEN_MODIFIERS},"full":True},"diagnosticProvider":{"interFileDependencies":False,"workspaceDiagnostics":False,"identifier":"sol"}}}}
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
        if ident is not None: return {"id":ident,"error":{"code":-32601,"message":"Method not found"}}
        return None

def send(msg):
    raw=json.dumps(msg,separators=(",",":")); sys.stdout.write(f"Content-Length: {len(raw.encode())}\r\n\r\n{raw}"); sys.stdout.flush()
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
