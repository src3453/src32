import json
import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote, urlparse

SERVER = Path(__file__).with_name("sol_language_server.py")
BUNDLED_SERVER = SERVER.parents[1] / "vscode-sol-syntax" / "server" / "sol_language_server.py"
WINDOWS_PATH_RE = re.compile(r"^/[A-Za-z]:")

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


NAV_LIBRARY = "\n".join([
    "!const LIMIT 8",
    "fn increment (value) : value 1 add ret ;",
    "fn reset () : local total 0 >total ret ;",
    "!var counter",
])

NAV_MAIN = "\n".join([
    '!include "lib.sol" as library',
    "!const GLOBAL 4",
    "fn main () : library.LIMIT library.increment GLOBAL add >library.counter ret ;",
])

SHADOW_SOURCE = "\n".join([
    "!const LIMIT 1",
    "!namespace inner -",
    "!const LIMIT 8",
    "fn pick () : LIMIT ret ;",
    "!end",
    "fn outer () : LIMIT ret ;",
])

def uri_to_path(uri):
    path = unquote(urlparse(uri).path)
    return Path(path[1:] if WINDOWS_PATH_RE.match(path) else path)

def location(uri, line, character, name):
    return uri, {"start": {"line": line, "character": character}, "end": {"line": line, "character": character + len(name)}}

def found_locations(result):
    if not result:
        return []
    if isinstance(result, dict):
        result = [result]
    return [(uri_to_path(item["location"]["uri"]), item["location"]["range"]) if "location" in item else (item["uri"], item["range"]) for item in result]

def found_ranges(result):
    return [(item["range"], item["kind"]) for item in result]

class Session:
    """Minimal LSP client driving one server process over stdio."""

    def __init__(self, server):
        self.process = subprocess.Popen([sys.executable, str(server)], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self.next_id = 1

    def __enter__(self):
        return self

    def __exit__(self, *_exc):
        try:
            self.request("shutdown", {})
        except (OSError, ValueError, AssertionError):
            pass
        try:
            self.notify("exit", {})
            self.process.wait(timeout=5)
        except (OSError, subprocess.TimeoutExpired):
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait(timeout=5)
        return None

    def notify(self, method, params):
        send(self.process.stdin, {"jsonrpc": "2.0", "method": method, "params": params})

    def request(self, method, params):
        ident = self.next_id
        self.next_id += 1
        send(self.process.stdin, {"jsonrpc": "2.0", "id": ident, "method": method, "params": params})
        while True:
            message = receive(self.process.stdout)
            if message.get("id") == ident:
                assert "error" not in message, message
                return message["result"]

    def initialize(self, root=None):
        return self.request("initialize", {"rootUri": root} if root else {})["capabilities"]

    def open(self, uri, text):
        self.notify("textDocument/didOpen", {"textDocument": {"uri": uri, "version": 1, "text": text}})

    def at(self, method, uri, line, character, **extra):
        params = {"textDocument": {"uri": uri}, "position": {"line": line, "character": character}}
        params.update(extra)
        return self.request(method, params)

def navigation_project(tmp_path):
    library = tmp_path / "lib.sol"
    library.write_text(NAV_LIBRARY, encoding="utf-8")
    main = tmp_path / "main.sol"
    main.write_text(NAV_MAIN, encoding="utf-8")
    return library, main

def test_lsp_navigates_declarations_across_includes(tmp_path):
    library, main = navigation_project(tmp_path)
    library_lines = NAV_LIBRARY.split("\n")
    main_lines = NAV_MAIN.split("\n")
    main_uri = main.as_uri()
    call = main_lines[2]
    for server in (SERVER, BUNDLED_SERVER):
        with Session(server) as session:
            capabilities = session.initialize(tmp_path.as_uri())
            for provider in ("definitionProvider", "declarationProvider", "referencesProvider", "hoverProvider", "documentHighlightProvider", "workspaceSymbolProvider"):
                assert capabilities[provider] is True, provider
            session.open(main_uri, NAV_MAIN)
            # the namespace qualifier names the aliased `!include`
            assert found_locations(session.at("textDocument/definition", main_uri, 2, call.index("library"))) == [
                location(main_uri, 0, main_lines[0].index("library"), "library")]
            # a constant of the included file
            assert found_locations(session.at("textDocument/declaration", main_uri, 2, call.index("LIMIT"))) == [
                location(library.as_uri(), 0, library_lines[0].index("LIMIT"), "LIMIT")]
            # a function of the included file
            assert found_locations(session.at("textDocument/definition", main_uri, 2, call.index("increment"))) == [
                location(library.as_uri(), 1, library_lines[1].index("increment"), "increment")]
            # a constant declared next to the reference
            assert found_locations(session.at("textDocument/definition", main_uri, 2, call.index("GLOBAL"))) == [
                location(main_uri, 1, main_lines[1].index("GLOBAL"), "GLOBAL")]
            # assignment to a qualified variable of the included file
            assert found_locations(session.at("textDocument/definition", main_uri, 2, call.index("counter"))) == [
                location(library.as_uri(), 3, library_lines[3].index("counter"), "counter")]
            hover = session.at("textDocument/hover", main_uri, 2, call.index("increment"))
            assert "fn increment (value)" in hover["contents"]["value"]
            assert "lib.sol" in hover["contents"]["value"]
            call_position = call.index("increment")
            assert found_locations(session.at("textDocument/references", main_uri, 2, call_position, context={"includeDeclaration": True})) == [
                location(library.as_uri(), 1, library_lines[1].index("increment"), "increment"),
                location(main_uri, 2, call_position, "increment")]
            assert found_locations(session.at("textDocument/references", main_uri, 2, call_position, context={"includeDeclaration": False})) == [
                location(main_uri, 2, call_position, "increment")]
            # a qualified reference is not visible from the included file itself
            assert session.at("textDocument/definition", main_uri, 1, main_lines[1].index("GLOBAL")) is not None
            assert found_locations(session.at("textDocument/references", library.as_uri(), 1, library_lines[1].index("increment"), context={"includeDeclaration": True})) == [
                location(library.as_uri(), 1, library_lines[1].index("increment"), "increment"),
                location(main_uri, 2, call_position, "increment")]

def test_lsp_navigates_arguments_locals_and_highlights(tmp_path):
    library, _main = navigation_project(tmp_path)
    library_uri = library.as_uri()
    lines = NAV_LIBRARY.split("\n")
    signature, store = lines[1], lines[2]
    argument = signature.index("value", signature.index(":"))
    write = store.index(">") + 1
    local = store.index("total")
    for server in (SERVER, BUNDLED_SERVER):
        with Session(server) as session:
            session.initialize(tmp_path.as_uri())
            session.open(library_uri, NAV_LIBRARY)
            assert found_locations(session.at("textDocument/definition", library_uri, 1, argument)) == [
                location(library_uri, 1, signature.index("value"), "value")]
            hover = session.at("textDocument/hover", library_uri, 1, argument)
            assert "argument value (of increment)" in hover["contents"]["value"]
            assert found_locations(session.at("textDocument/definition", library_uri, 2, write)) == [
                location(library_uri, 2, local, "total")]
            assert found_locations(session.at("textDocument/definition", library_uri, 2, local)) == [
                location(library_uri, 2, local, "total")]
            assert found_ranges(session.at("textDocument/documentHighlight", library_uri, 2, write)) == [
                ({"start": {"line": 2, "character": local}, "end": {"line": 2, "character": local + 5}}, 1),
                ({"start": {"line": 2, "character": write}, "end": {"line": 2, "character": write + 5}}, 3)]
            assert found_locations(session.at("textDocument/references", library_uri, 2, write, context={"includeDeclaration": True})) == [
                location(library_uri, 2, local, "total"), location(library_uri, 2, write, "total")]
            # an argument stays inside its own function
            assert found_locations(session.at("textDocument/references", library_uri, 1, argument, context={"includeDeclaration": True})) == [
                location(library_uri, 1, signature.index("value"), "value"), location(library_uri, 1, argument, "value")]

def test_lsp_prefers_namespaced_declarations_and_tolerates_broken_buffers(tmp_path):
    uri = "file:///tmp/navigation-shadow.sol"
    lines = SHADOW_SOURCE.split("\n")
    pick, outer = lines[3], lines[5]
    for server in (SERVER, BUNDLED_SERVER):
        with Session(server) as session:
            session.initialize(tmp_path.as_uri())
            session.open(uri, SHADOW_SOURCE)
            # inside `!namespace inner -` the namespace constant shadows the global one
            assert found_locations(session.at("textDocument/definition", uri, 3, pick.index("LIMIT", pick.index(":")))) == [
                (uri, {"start": {"line": 2, "character": lines[2].index("LIMIT")}, "end": {"line": 2, "character": lines[2].index("LIMIT") + 5}})]
            assert found_locations(session.at("textDocument/definition", uri, 5, outer.index("LIMIT", outer.index(":")))) == [
                (uri, {"start": {"line": 0, "character": lines[0].index("LIMIT")}, "end": {"line": 0, "character": lines[0].index("LIMIT") + 5}})]
            # an unterminated string is a normal in-flight buffer: navigation answers null
            broken = "file:///tmp/navigation-broken.sol"
            session.open(broken, 'fn main () : "unterminated\n')
            assert session.at("textDocument/definition", broken, 0, 4) is None
            assert session.at("textDocument/hover", broken, 0, 4) is None
            assert session.request("textDocument/semanticTokens/full", {"textDocument": {"uri": broken}})["data"] == []
            assert [item["name"] for item in session.request("textDocument/documentSymbol", {"textDocument": {"uri": broken}})] == ["main"]

def test_lsp_lists_workspace_symbols(tmp_path):
    library, main = navigation_project(tmp_path)
    library_lines = NAV_LIBRARY.split("\n")
    for server in (SERVER, BUNDLED_SERVER):
        with Session(server) as session:
            session.initialize(tmp_path.as_uri())
            symbols = found_locations(session.request("workspace/symbol", {"query": "infix"}))
            assert symbols == []
            symbols = session.request("workspace/symbol", {"query": "incre"})
            assert [(uri_to_path(item["location"]["uri"]), item["name"], item["kind"], item["location"]["range"]) for item in symbols] == [
                (library, "increment", 12, {"start": {"line": 1, "character": library_lines[1].index("increment")}, "end": {"line": 1, "character": library_lines[1].index("increment") + 9}})]
            names = {item["name"] for item in session.request("workspace/symbol", {"query": ""})}
            assert {"LIMIT", "increment", "reset", "counter", "GLOBAL", "main", "library"} <= names
            assert not names & {"total", "value"}
        with Session(server) as session:
            session.initialize()
            session.open(main.as_uri(), NAV_MAIN)
            names = {item["name"] for item in session.request("workspace/symbol", {"query": ""})}
            assert "main" in names
            assert "increment" not in names


NESTED_SOURCE = "\n".join([
    "!namespace outer -",
    "!const BASE 10",
    "!namespace inner -",
    "!const BASE 20",
    "fn pick () : BASE ret ;",
    "!end",
    "fn local_pick () : BASE ret ;",
    "!end",
    "outer.inner.pick",
    "outer.local_pick",
])


def test_lsp_reports_nested_namespaces_outline_tokens_and_navigation(tmp_path):
    source_path = tmp_path / "nested.sol"
    source_path.write_text(NESTED_SOURCE, encoding="utf-8")
    uri = source_path.as_uri()
    lines = NESTED_SOURCE.split("\n")
    call = lines[8]
    member = call.index("pick")
    for server in (SERVER, BUNDLED_SERVER):
        with Session(server) as session:
            capabilities = session.initialize(tmp_path.as_uri())
            session.open(uri, NESTED_SOURCE)
            symbols = session.request("textDocument/documentSymbol", {"textDocument": {"uri": uri}})
            assert [(item["name"], item["kind"]) for item in symbols] == [("outer", 3)]
            outer = symbols[0]
            assert [(item["name"], item["kind"]) for item in outer["children"]] == [
                ("BASE", 14), ("inner", 3), ("local_pick", 12)]
            inner = outer["children"][1]
            assert [(item["name"], item["kind"]) for item in inner["children"]] == [("BASE", 14), ("pick", 12)]
            assert inner["range"]["end"] == {"line": 5, "character": len(lines[5])}
            assert outer["range"]["end"] == {"line": 7, "character": len(lines[7])}
            # every namespace component navigates to the namespace it names
            assert found_locations(session.at("textDocument/definition", uri, 8, call.index("outer"))) == [
                location(uri, 0, lines[0].index("outer"), "outer")]
            assert found_locations(session.at("textDocument/definition", uri, 8, call.index("inner"))) == [
                location(uri, 2, lines[2].index("inner"), "inner")]
            assert found_locations(session.at("textDocument/definition", uri, 8, member)) == [
                location(uri, 4, lines[4].index("pick"), "pick")]
            hover = session.at("textDocument/hover", uri, 8, member)
            assert "fn pick ()" in hover["contents"]["value"]
            legend = capabilities["semanticTokensProvider"]["legend"]["tokenTypes"]
            tokens = session.request("textDocument/semanticTokens/full", {"textDocument": {"uri": uri}})["data"]
            marked = decode_semantic_tokens(tokens, legend)
            assert marked[(8, call.index("outer"))] == ("namespace", len("outer.inner"))
            assert marked[(8, member)] == ("function", len("pick"))
            assert marked[(9, lines[9].index("outer"))] == ("namespace", 5)
            assert marked[(9, lines[9].index("local_pick"))] == ("function", len("local_pick"))


def test_lsp_analyzes_nested_namespace_scopes(tmp_path):
    bundled_server = SERVER.parents[1] / "vscode-sol-syntax" / "server" / "sol_language_server.py"
    nested = "\n".join([
        "!namespace outer -",
        "!const BASE 10",
        "!namespace inner -",
        "fn pick () : BASE ret ;",
        "!end",
        "!end",
        "outer.inner.pick",
    ])
    broken = "\n".join([
        "!namespace outer -",
        "!namespace inner -",
        "!const K 1",
        "!end",
        "!end",
        "outer.inner.MISSING",
    ])
    analyze = (
        "import json,sys; sys.path.insert(0,sys.argv[1]); "
        "from sol_language_server import Server; "
        "print(json.dumps(Server().analyze(sys.argv[2],sys.argv[3])))"
    )

    for server in (SERVER, bundled_server):
        def run(source):
            result = subprocess.run(
                [sys.executable, "-c", analyze, str(server.parent), "file:///tmp/nested.sol", source],
                capture_output=True,
                text=True,
                stdin=subprocess.DEVNULL,
                check=True,
            )
            return json.loads(result.stdout)

        assert run(nested) == []
        diagnostics = run(broken)
        assert [(item["severity"], item["message"], item["range"]["start"]) for item in diagnostics] == [
            (1, "unknown word: outer.inner.MISSING", {"line": 5, "character": 0})]


def test_lsp_completes_and_diagnoses_abi_compat(tmp_path):
    for server in (SERVER, BUNDLED_SERVER):
        lib = tmp_path / "lib.sol"
        lib.write_text('!abi_compat "Foundation"\nfn f () : 1 ret ;\n', encoding="utf-8")
        main = tmp_path / "main.sol"
        uri = main.as_uri()
        p = subprocess.Popen([sys.executable, str(server)], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        send(p.stdin, {"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{}})
        assert "completionProvider" in receive(p.stdout)["result"]["capabilities"]
        text = '!abi_compat "F\n'
        send(p.stdin, {"jsonrpc":"2.0", "method":"textDocument/didOpen", "params":{"textDocument":{"uri":uri,"version":1,"text":text}}})
        receive(p.stdout)
        send(p.stdin, {"jsonrpc":"2.0", "id":2, "method":"textDocument/completion", "params":{"textDocument":{"uri":uri},"position":{"line":0,"character":14}}})
        items = receive(p.stdout)["result"]
        assert [item["textEdit"]["newText"] for item in items] == ['"Foundation"']
        assert items[0]["textEdit"]["range"]["start"]["character"] == 12
        send(p.stdin, {"jsonrpc":"2.0", "id":3, "method":"textDocument/completion", "params":{"textDocument":{"uri":uri},"position":{"line":0,"character":4}}})
        assert [item["label"] for item in receive(p.stdout)["result"]] == ["!abi_compat"]
        for source, expected_line, message in (
            ('1 drop\n!abi_compat "Foundation"\n', 1, "must appear at the top level"),
            ('!abi_compat "Foundation"\nfn irq3 () : retn ;\n', 1, "IRQ handler functions"),
            ('!include "lib.sol"\nf drop\n', 0, str(lib)),
        ):
            send(p.stdin, {"jsonrpc":"2.0", "method":"textDocument/didChange", "params":{"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":source}]}})
            diagnostics = receive(p.stdout)["params"]["diagnostics"]
            assert message in diagnostics[0]["message"]
            assert diagnostics[0]["range"]["start"]["line"] == expected_line
        send(p.stdin, {"jsonrpc":"2.0", "id":4, "method":"shutdown", "params":{}})
        assert receive(p.stdout)["result"] is None
        send(p.stdin, {"jsonrpc":"2.0", "method":"exit", "params":{}})
        p.wait(timeout=10)
