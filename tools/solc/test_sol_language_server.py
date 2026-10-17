import json
import subprocess
import sys
from pathlib import Path

SERVER = Path(__file__).with_name("sol_language_server.py")

def send(stream, message):
    raw = json.dumps(message, separators=(",", ":")).encode()
    stream.write(f"Content-Length: {len(raw)}\r\n\r\n".encode() + raw)
    stream.flush()

def receive(stream):
    headers = {}
    while True:
        line = stream.readline()
        if line == b"\r\n": break
        key, value = line.decode().split(":", 1)
        headers[key.lower()] = value.strip()
    return json.loads(stream.read(int(headers["content-length"])))

def test_server_initialize_diagnostics_and_tokens():
    p = subprocess.Popen([sys.executable, str(SERVER)], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    send(p.stdin, {"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{}})
    init = receive(p.stdout)
    assert init["result"]["capabilities"]["semanticTokensProvider"]["full"] is True
    uri = "file:///tmp/example.sol"
    send(p.stdin, {"jsonrpc":"2.0", "method":"textDocument/didOpen", "params":{"textDocument":{"uri":uri,"version":1,"text":"drop"}}})
    published = receive(p.stdout)
    assert "stack underflow" in published["params"]["diagnostics"][0]["message"]
    send(p.stdin, {"jsonrpc":"2.0", "id":2, "method":"textDocument/semanticTokens/full", "params":{"textDocument":{"uri":uri}}})
    tokens = receive(p.stdout)
    assert tokens["result"]["data"]
    send(p.stdin, {"jsonrpc":"2.0", "id":3, "method":"shutdown", "params":{}})
    assert receive(p.stdout)["result"] is None
    send(p.stdin, {"jsonrpc":"2.0", "method":"exit", "params":{}})
    p.wait(timeout=3)


def test_lsp_accepts_assignment_to_function_arguments():
    bundled_server = SERVER.parents[1] / "vscode-sol-syntax" / "server" / "sol_language_server.py"
    source = "fn set (value) : 42 >value value ret ; 1 set"
    for server in (SERVER, bundled_server):
        process = subprocess.Popen([sys.executable, str(server)], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        try:
            send(process.stdin, {"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{}})
            receive(process.stdout)
            uri = "file:///tmp/argument-assignment.sol"
            send(process.stdin, {"jsonrpc":"2.0", "method":"textDocument/didOpen", "params":{"textDocument":{"uri":uri,"version":1,"text":source}}})
            published = receive(process.stdout)
            assert published["params"]["diagnostics"] == []
            send(process.stdin, {"jsonrpc":"2.0", "id":2, "method":"shutdown", "params":{}})
            receive(process.stdout)
            send(process.stdin, {"jsonrpc":"2.0", "method":"exit", "params":{}})
            process.wait(timeout=3)
        finally:
            if process.poll() is None:
                process.kill()

def test_lsp_accepts_namespace_blocks_and_aliased_includes(tmp_path):
    bundled_server = SERVER.parents[1] / "vscode-sol-syntax" / "server" / "sol_language_server.py"
    library = tmp_path / "library.sol"
    library.write_text(
        "fn increment (value) : value 1 add ret ;\n"
        "fn answer () : 41 increment ret ;\n",
        encoding="utf-8",
    )
    source = f'!include "{library.as_posix()}" as lib\nlib.answer'
    analyze = (
        "import json,sys; sys.path.insert(0,sys.argv[1]); "
        "from sol_language_server import Server; "
        "print(json.dumps(Server().analyze(sys.argv[2],sys.argv[3])))"
    )

    for server in (SERVER, bundled_server):
        result = subprocess.run(
            [sys.executable, "-c", analyze, str(server.parent), "file:///tmp/namespace.sol", source],
            capture_output=True,
            text=True,
            stdin=subprocess.DEVNULL,
            check=True,
        )
        assert json.loads(result.stdout) == []


NAMESPACE_SOURCE = "\n".join([
    "!namespace lib -",
    "!const LIMIT 8",
    "!var counter",
    "fn add (a b) : a b add ret ;",
    "fn reset () :",
    "  local tmp",
    "  0 >counter",
    "  retn",
    ";",
    "!end",
    "!asm",
    "; fn ghost () : nothing ;",
    "!end",
    '!const MSG "fn hidden"',
    "!define MAX 10",
    "fn main () : 1 lib.add ret ;",
    '!include "missing.sol" as other',
])


def decode_semantic_tokens(data, legend):
    line = column = 0
    decoded = {}
    for index in range(0, len(data), 5):
        delta_line, delta_start, length, kind, _ = data[index:index + 5]
        line += delta_line
        column = delta_start if delta_line else column + delta_start
        decoded[(line, column)] = (legend[kind], length)
    return decoded


def test_lsp_reports_namespace_outline_and_semantic_tokens():
    bundled_server = SERVER.parents[1] / "vscode-sol-syntax" / "server" / "sol_language_server.py"
    uri = "file:///tmp/outline.sol"
    lines = NAMESPACE_SOURCE.split("\n")
    for server in (SERVER, bundled_server):
        process = subprocess.Popen([sys.executable, str(server)], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        try:
            send(process.stdin, {"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{}})
            initialize = receive(process.stdout)
            assert initialize["result"]["capabilities"]["documentSymbolProvider"] is True
            legend = initialize["result"]["capabilities"]["semanticTokensProvider"]["legend"]["tokenTypes"]
            send(process.stdin, {"jsonrpc":"2.0", "method":"textDocument/didOpen", "params":{"textDocument":{"uri":uri,"version":1,"text":NAMESPACE_SOURCE}}})
            receive(process.stdout)
            send(process.stdin, {"jsonrpc":"2.0", "id":2, "method":"textDocument/documentSymbol", "params":{"textDocument":{"uri":uri}}})
            symbols = receive(process.stdout)["result"]
            send(process.stdin, {"jsonrpc":"2.0", "id":3, "method":"textDocument/semanticTokens/full", "params":{"textDocument":{"uri":uri}}})
            tokens = receive(process.stdout)["result"]["data"]
            send(process.stdin, {"jsonrpc":"2.0", "id":4, "method":"shutdown", "params":{}})
            receive(process.stdout)
            send(process.stdin, {"jsonrpc":"2.0", "method":"exit", "params":{}})
            process.wait(timeout=3)
        finally:
            if process.poll() is None:
                process.kill()
        assert [(symbol["name"], symbol["kind"]) for symbol in symbols] == [("lib", 3), ("MSG", 14), ("MAX", 14), ("main", 12)]
        library = symbols[0]
        assert [(child["name"], child["kind"], child.get("detail")) for child in library["children"]] == [
            ("LIMIT", 14, None), ("counter", 13, None), ("add", 12, "(a b)"), ("reset", 12, "()")]
        assert [local["name"] for local in library["children"][3]["children"]] == ["tmp"]
        assert library["range"] == {"start": {"line": 0, "character": 0}, "end": {"line": 9, "character": 4}}
        assert library["children"][2]["selectionRange"] == {"start": {"line": 3, "character": 3}, "end": {"line": 3, "character": 6}}
        assert library["children"][2]["range"]["end"] == {"line": 3, "character": len(lines[3])}
        assert library["children"][3]["range"]["end"] == {"line": 8, "character": 1}
        assert symbols[1]["selectionRange"]["start"] == {"line": 13, "character": 7}
        assert (symbols[2]["name"], symbols[2]["kind"], symbols[2]["range"]["start"]["line"]) == ("MAX", 14, 14)
        assert symbols[2]["selectionRange"]["start"] == {"line": 14, "character": 8}
        assert symbols[3]["range"] == {"start": {"line": 15, "character": 0}, "end": {"line": 15, "character": len(lines[15])}}
        pending = list(symbols)
        while pending:
            current = pending.pop()
            start = (current["range"]["start"]["line"], current["range"]["start"]["character"])
            end = (current["range"]["end"]["line"], current["range"]["end"]["character"])
            selection_start = (current["selectionRange"]["start"]["line"], current["selectionRange"]["start"]["character"])
            selection_end = (current["selectionRange"]["end"]["line"], current["selectionRange"]["end"]["character"])
            assert start <= selection_start <= selection_end <= end
            pending.extend(current.get("children", []))
        marked = decode_semantic_tokens(tokens, legend)
        assert marked[(0, lines[0].index("lib"))] == ("namespace", 3)
        assert marked[(1, lines[1].index("LIMIT"))] == ("constant", 5)
        assert marked[(15, lines[15].index("lib"))] == ("namespace", 3)
        assert marked[(15, lines[15].index("add"))] == ("function", 3)
        assert marked[(16, lines[16].index("other"))] == ("namespace", 5)
        assert marked[(10, 0)] == ("directive", 4)
        assert not any(line in (8, 11) for line, _ in marked)
