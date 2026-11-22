"""Tests for the CNET TCP bridge (doc/spec_NET_bridge.md).

    python -m unittest discover -s tools/cnet-bridge -v

A Python client stands in for Foundation's telnet: it is a CNET node on the
far end of a localhost UDP segment and opens an L3P stream
(doc/spec_NET_L3P.md §6) to the bridge, with optional packet loss. The SSH
tests need paramiko (they skip without it).
"""
import socket
import struct
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / 'l3p'))
import cnet_bridge as cb  # noqa: E402
import l3p  # noqa: E402
from test_l3p import Lossy  # noqa: E402

try:
    import paramiko
except ImportError:
    paramiko = None

NET = 1
CLIENT_NODE = 0x0010
CLIENT_SOCKET = 200


class RawNode:
    """Sends raw link frames to the bridge and records what comes back."""

    def __init__(self, bridge_port):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(('127.0.0.1', 0))
        self.sock.settimeout(0.02)
        self.bridge_port = bridge_port
        self.sender = 0x0BADF00D
        self.datagrams = []  # (src_node, src_socket, dst_socket, proto, body)
        self.links = []      # (kind, dst, src)

    def close(self):
        self.sock.close()

    def send_link(self, frame):
        packet = b'CNET' + struct.pack('>BBHI', 1, 0, 0, self.sender) + frame
        self.sock.sendto(packet, ('127.0.0.1', self.bridge_port))

    def send_datagram(self, dst_socket, proto, payload, dst_node=cb.BRIDGE_NODE, dst_net=0):
        body = l3p.datagram(dst_net, dst_node, NET, CLIENT_NODE, dst_socket, CLIENT_SOCKET, proto, payload)
        self.send_link(l3p.link_frame(dst_node, CLIENT_NODE, l3p.LINK_DATAGRAM, body))

    def poll(self):
        try:
            packet, _ = self.sock.recvfrom(4096)
        except (socket.timeout, OSError):
            return False
        frame = packet[12:]
        dst, src, kind, _, _ = struct.unpack('>HHBBH', frame[:8])
        self.links.append((kind, dst, src))
        if kind == l3p.LINK_DATAGRAM:
            parsed = l3p.parse_datagram(frame[8:])
            if parsed:
                dnet, dnode, snet, snode, dsock, ssock, proto, body = parsed
                self.datagrams.append((snode, ssock, dsock, proto, body))
        return True

    def run_until(self, done, timeout=10.0):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline and not done():
            self.poll()
        return done()


class Client:
    """A CNET node that opens one L3P stream to the bridge, sends the target
    line and keeps what the bridge sends after its OK line."""

    def __init__(self, bridge_port, loss=0.0):
        self.segment = l3p.Segment(('127.0.0.1', 0), peers=[('127.0.0.1', bridge_port)])
        if loss:
            self.segment.mangle = Lossy(None, drop=loss)
        self.node = l3p.Node(self.segment, NET, CLIENT_NODE)
        self.segment.start()
        self.endpoint = l3p.Endpoint(self.node)
        self.assoc = None
        self.state = 'closed'
        self.reset_reason = None
        self.received = bytearray()
        self.rcv_fin = False

    def close(self):
        self.endpoint.close()
        self.segment.close()

    @property
    def fin_acked(self):
        return bool(self.assoc and self.assoc.fin_acked)

    def open(self, target):
        try:
            self.assoc = self.endpoint.open_stream(NET, cb.BRIDGE_NODE, cb.BRIDGE_SOCKET)
            self.assoc.write(target.encode() + b'\n')
            line = bytearray()
            while b'\n' not in line:
                chunk = self.assoc.read(timeout=20)
                if not chunk:
                    self.state = 'closed'
                    return self.state
                line += chunk
        except l3p.L3PError as error:
            self.state = 'reset'
            self.reset_reason = error.message
            return self.state
        first, _, rest = bytes(line).partition(b'\n')
        if first != b'OK':
            self.state = 'bad answer'
            return self.state
        self.received += rest
        self.state = 'open'
        threading.Thread(target=self.reader, daemon=True).start()
        return self.state

    def reader(self):
        try:
            while True:
                chunk = self.assoc.read()
                if not chunk:
                    self.rcv_fin = True
                    return
                self.received += chunk
        except l3p.L3PError as error:
            self.state = 'reset'
            self.reset_reason = error.message

    def write(self, data):
        self.assoc.write(data)

    def close_write(self):
        self.assoc.close_write()

    def run_until(self, done, timeout=10.0):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline and not done():
            time.sleep(0.02)
        return done()


class TcpServer:
    """A TCP server whose handler runs in a thread per connection."""

    def __init__(self, handler):
        self.sock = socket.socket()
        self.sock.bind(('127.0.0.1', 0))
        self.sock.listen()
        self.port = self.sock.getsockname()[1]
        self.handler = handler
        self.seen = []
        threading.Thread(target=self.run, daemon=True).start()

    def run(self):
        while True:
            try:
                conn, _ = self.sock.accept()
            except OSError:
                return
            threading.Thread(target=self.handler, args=(self, conn), daemon=True).start()

    def close(self):
        self.sock.close()


def echo_upper(server, conn):
    conn.sendall(b'welcome\r\n')
    while True:
        try:
            data = conn.recv(4096)
        except OSError:  # the bridge dropped the connection
            break
        if not data:
            break
        server.seen.append(data)
        if data.strip() == b'big':
            conn.sendall(bytes(range(256)) * 400)  # includes 0xFF bytes, passed unchanged
            conn.sendall(b'END')
            conn.close()
            return
        conn.sendall(data.upper())
    conn.close()


class BridgeHarness(unittest.TestCase):
    allow = ['127.0.0.1']

    def setUp(self):
        self.segment = cb.Segment(('127.0.0.1', 0), 0)
        self.node = cb.Node(self.segment, NET, cb.BRIDGE_NODE)
        self.tmp = tempfile.TemporaryDirectory()
        self.bridge = cb.Bridge(self.node, self.allow, known_hosts=Path(self.tmp.name) / 'known_hosts')
        self.segment.start()
        self.clients = []

    def tearDown(self):
        for client in self.clients:
            client.close()
        self.bridge.close()
        self.segment.close()
        self.tmp.cleanup()

    def client(self, **kw):
        client = Client(self.segment.port, **kw)
        self.clients.append(client)
        return client

    def raw(self):
        node = RawNode(self.segment.port)
        self.clients.append(node)
        return node


class NodeTests(BridgeHarness):
    def test_echo_and_probe(self):
        c = self.raw()
        c.send_datagram(l3p.ECHO_SOCKET, l3p.PROTO_ECHO, b'\x01ping')
        c.send_link(l3p.link_frame(cb.BRIDGE_NODE, 0, l3p.LINK_PROBE, b'abcd'))
        c.run_until(lambda: c.datagrams and any(k == l3p.LINK_IN_USE for k, _, _ in c.links), 3)
        self.assertEqual(c.datagrams[0], (cb.BRIDGE_NODE, l3p.ECHO_SOCKET, CLIENT_SOCKET, l3p.PROTO_ECHO, b'\x02ping'))
        self.assertIn((l3p.LINK_IN_USE, cb.BRIDGE_NODE, cb.BRIDGE_NODE), c.links)

    def test_learns_the_client_as_a_peer(self):
        # The bridge has no --peer: it answers the address that sent the frame.
        c = self.raw()
        c.send_datagram(l3p.ECHO_SOCKET, l3p.PROTO_ECHO, b'\x01x')
        self.assertTrue(c.run_until(lambda: c.datagrams, 3))


class TelnetTests(BridgeHarness):
    def setUp(self):
        super().setUp()
        self.server = TcpServer(echo_upper)

    def tearDown(self):
        self.server.close()
        super().tearDown()

    def test_relay_both_ways_and_close(self):
        c = self.client()
        self.assertEqual(c.open(f'telnet://127.0.0.1:{self.server.port}'), 'open')
        self.assertTrue(c.run_until(lambda: b'welcome\r\n' in c.received))
        c.write(b'hello\xff\r\n')
        self.assertTrue(c.run_until(lambda: b'HELLO\xff\r\n' in c.received))
        self.assertIn(b'hello\xff\r\n', self.server.seen)  # bytes pass unchanged, IAC included
        c.close_write()
        self.assertTrue(c.run_until(lambda: c.rcv_fin and c.fin_acked))

    def test_bulk_transfer_survives_packet_loss(self):
        c = self.client(loss=0.2)
        self.assertEqual(c.open(f'telnet://127.0.0.1:{self.server.port}'), 'open')
        c.write(b'big\r\n')
        self.assertTrue(c.run_until(lambda: c.rcv_fin, 60), len(c.received))
        self.assertEqual(bytes(c.received), b'welcome\r\n' + bytes(range(256)) * 400 + b'END')

    def test_target_not_allowed(self):
        c = self.client()
        self.assertEqual(c.open('telnet://example.org'), 'reset')
        self.assertIn('not allowed', c.reset_reason)

    def test_connection_refused(self):
        s = socket.socket()
        s.bind(('127.0.0.1', 0))
        port = s.getsockname()[1]
        s.close()
        c = self.client()
        self.assertEqual(c.open(f'telnet://127.0.0.1:{port}'), 'reset')
        self.assertIn(f'127.0.0.1:{port}', c.reset_reason)

    def test_bad_target_and_unknown_connection(self):
        c = self.client()
        self.assertEqual(c.open('gopher://127.0.0.1'), 'reset')
        self.assertIn('bad target', c.reset_reason)
        raw = self.raw()
        header = l3p.HEADER.pack(0x12, l3p.T_DATA, 0, 0, 0x1234, 0, 0, 0, 0)
        raw.send_datagram(cb.BRIDGE_SOCKET, l3p.PROTO_L3P, header + b'x')
        self.assertTrue(raw.run_until(lambda: raw.datagrams, 3))
        body = raw.datagrams[0][4]
        self.assertEqual((body[1], l3p.parse_reset(body[20:])[0]), (l3p.T_RESET, l3p.R_UNKNOWN_CONN))

    def test_target_line_is_required(self):
        c = self.client()
        c.assoc = c.endpoint.open_stream(NET, cb.BRIDGE_NODE, cb.BRIDGE_SOCKET)
        c.assoc.close_write()  # EOF before a target line
        with self.assertRaises(l3p.L3PError) as cm:
            c.assoc.read(timeout=10)
        self.assertEqual(cm.exception.reason, l3p.R_REFUSED)

    def test_one_association_per_session(self):
        c = self.client()
        self.assertEqual(c.open(f'telnet://127.0.0.1:{self.server.port}'), 'open')
        c.write(b'abc')
        self.assertTrue(c.run_until(lambda: b'ABC' in c.received))
        self.assertEqual(self.bridge.endpoint.count(), 1)


class PureTests(unittest.TestCase):
    def test_parse_target(self):
        self.assertEqual(cb.parse_target('telnet://host'), ('telnet', 'host', 23, None))
        self.assertEqual(cb.parse_target('ssh://me@host:2222'), ('ssh', 'host', 2222, 'me'))
        self.assertEqual(cb.parse_target('ssh://[::1]:22'), ('ssh', '::1', 22, None))
        for bad in ('host', 'telnet://me@host', 'ftp://h', 'ssh://', 'telnet://h:0', 'telnet://a b'):
            with self.assertRaises(ValueError, msg=bad):
                cb.parse_target(bad)

    def test_allowed(self):
        rules = ['localhost', '*.example.org:22', '10.0.0.5:2?']
        self.assertTrue(cb.allowed(rules, 'LOCALHOST', 23))
        self.assertTrue(cb.allowed(rules, 'a.example.org', 22))
        self.assertFalse(cb.allowed(rules, 'a.example.org', 23))
        self.assertTrue(cb.allowed(rules, '10.0.0.5', 23))
        self.assertFalse(cb.allowed(rules, '10.0.0.6', 23))
        self.assertTrue(cb.allowed(['*'], 'x', 1))
        self.assertFalse(cb.allowed([], 'x', 1))

    def test_telnet_server_parses_options(self):
        sent = []
        t = cb.TelnetServer(sent.append)
        sizes = []
        t.on_resize = lambda c, r: sizes.append((c, r))
        data = bytes([cb.IAC, cb.WILL, cb.OPT_NAWS, cb.IAC, cb.SB, cb.OPT_NAWS, 0, 80, 0, 30, cb.IAC, cb.SE,
                      cb.IAC, cb.WILL, cb.OPT_TTYPE]) + b'ls\r\0x\r\ny\xff\xff' + bytes([cb.IAC, cb.DO, 99])
        self.assertEqual(t.feed(data), b'ls\rx\ry\xff')
        self.assertEqual(sizes, [(80, 30)])
        self.assertIn(bytes([cb.IAC, cb.SB, cb.OPT_TTYPE, cb.TTYPE_SEND, cb.IAC, cb.SE]), sent)
        self.assertIn(bytes([cb.IAC, cb.WONT, 99]), sent)
        t.feed(bytes([cb.IAC, cb.SB, cb.OPT_TTYPE, cb.TTYPE_IS]) + b'ANSI' + bytes([cb.IAC, cb.SE]))
        self.assertEqual(t.term, 'ansi')


class SshServer(TcpServer):
    """A paramiko SSH server: user cpt, password secret. The shell echoes
    input (CR becomes CR LF) and says bye after "exit" and CR. pty gets the
    PTY requests and window changes, typed the raw input."""

    _key = None

    def __init__(self):
        if SshServer._key is None:
            SshServer._key = paramiko.RSAKey.generate(2048)
        self.host_key = SshServer._key
        self.pty = []
        self.typed = bytearray()
        super().__init__(SshServer.serve)

    def serve(self, conn):
        server = self

        class Interface(paramiko.ServerInterface):
            def check_auth_password(self, username, password):
                ok = (username, password) == ('cpt', 'secret')
                return paramiko.AUTH_SUCCESSFUL if ok else paramiko.AUTH_FAILED

            def get_allowed_auths(self, username):
                return 'password'

            def check_channel_request(self, kind, chanid):
                return paramiko.OPEN_SUCCEEDED

            def check_channel_pty_request(self, channel, term, width, height, pw, ph, modes):
                server.pty.append((term.decode() if isinstance(term, bytes) else term, width, height))
                return True

            def check_channel_shell_request(self, channel):
                return True

            def check_channel_window_change_request(self, channel, width, height, pw, ph):
                server.pty.append(('resize', width, height))
                return True

        transport = paramiko.Transport(conn)
        transport.add_server_key(self.host_key)
        try:
            transport.start_server(server=Interface())
        except paramiko.SSHException:
            return
        channel = transport.accept(20)
        if channel is None:
            return
        channel.sendall(b'$ ')
        while True:
            data = channel.recv(1024)
            if not data:
                break
            self.typed += data
            if b'exit\r' in self.typed:
                channel.sendall(b'\r\nbye\r\n')
                break
            channel.sendall(data.replace(b'\r', b'\r\n'))
        channel.send_exit_status(0)
        channel.close()
        transport.close()


@unittest.skipUnless(paramiko, 'paramiko is not installed')
class SshTests(BridgeHarness):
    def setUp(self):
        super().setUp()
        self.server = SshServer()
        self.pty = self.server.pty
        self.typed = self.server.typed

    def tearDown(self):
        self.server.close()
        super().tearDown()

    def answer(self, c, prompt, text):
        self.assertTrue(c.run_until(lambda: prompt in c.received, 20), bytes(c.received))
        c.write(text)

    def test_login_shell_and_exit(self):
        c = self.client()
        self.assertEqual(c.open(f'ssh://127.0.0.1:{self.server.port}'), 'open')
        # The bridge's telnet negotiation, then the client's replies.
        self.assertTrue(c.run_until(lambda: bytes([cb.IAC, cb.DO, cb.OPT_TTYPE]) in c.received))
        c.write(bytes([cb.IAC, cb.WILL, cb.OPT_NAWS, cb.IAC, cb.SB, cb.OPT_NAWS, 0, 80, 0, 30, cb.IAC, cb.SE,
                       cb.IAC, cb.WILL, cb.OPT_TTYPE]))
        self.assertTrue(c.run_until(lambda: bytes([cb.IAC, cb.SB, cb.OPT_TTYPE, cb.TTYPE_SEND]) in c.received))
        c.write(bytes([cb.IAC, cb.SB, cb.OPT_TTYPE, cb.TTYPE_IS]) + b'ANSI' + bytes([cb.IAC, cb.SE]))
        self.answer(c, b'(yes/no)? ', b'yes\r\0')
        self.answer(c, b'login as: ', b'cpt\r\0')
        self.answer(c, b"cpt@127.0.0.1's password: ", b'wrong\r\0')
        self.answer(c, b'try again', b'')
        self.answer(c, b"password: ", b'secret\r\0')
        self.assertTrue(c.run_until(lambda: b'$ ' in c.received, 20))
        self.assertNotIn(b'secret', bytes(c.received))  # the password is not echoed
        self.assertEqual(self.pty[0], ('ansi', 80, 30))
        c.write(bytes([cb.IAC, cb.SB, cb.OPT_NAWS, 0, 40, 0, 30, cb.IAC, cb.SE]))
        c.write(b'ls\r\0exit\r\n')
        self.assertTrue(c.run_until(lambda: c.rcv_fin, 20), bytes(c.received))
        self.assertIn(b'bye', bytes(c.received))
        self.assertEqual(bytes(self.typed), b'ls\rexit\r')  # CR NUL and CR LF arrive as CR
        self.assertIn(('resize', 40, 30), self.pty)
        known = (Path(self.tmp.name) / 'known_hosts').read_text()
        self.assertIn(f'[127.0.0.1]:{self.server.port} ssh-rsa ', known)

    def test_user_in_target_and_host_key_refused(self):
        c = self.client()
        self.assertEqual(c.open(f'ssh://cpt@127.0.0.1:{self.server.port}'), 'open')
        self.answer(c, b'(yes/no)? ', b'no\r\0')
        self.assertTrue(c.run_until(lambda: c.rcv_fin, 20))
        self.assertNotIn(b'password', bytes(c.received))


if __name__ == '__main__':
    unittest.main()
