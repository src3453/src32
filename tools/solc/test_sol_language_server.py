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
