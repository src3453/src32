"""Tests of the host L3P module (doc/spec_NET_L3P.md §14) over localhost UDP.

    python -m unittest discover -s tools/l3p -v
"""
import hashlib
import random
import struct
import sys
import threading
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import l3p  # noqa: E402


class Lossy:
    """A Segment.mangle that drops, duplicates and reorders packets.

    seed=None draws from OS entropy. A fixed seed is reproducible but was
    seen to drop every retransmission of one request in a long pipelined
    run (the retransmission rounds fall into a fixed rhythm), so long bulk
    tests use seed=None."""

    def __init__(self, seed, drop=0.0, dup=0.0, reorder=0.0, corrupt=0.0):
        self.rng = random.Random(seed)
        self.drop, self.dup, self.reorder, self.corrupt = drop, dup, reorder, corrupt
        self.held = None
        self.lock = threading.Lock()

    def __call__(self, packet):
        with self.lock:
            out = []
            if self.held is not None:
                held, self.held = self.held, None
            else:
                held = None
            if self.rng.random() < self.drop:
                pass
            elif self.rng.random() < self.reorder and held is None:
                self.held = packet
            else:
                if self.rng.random() < self.corrupt and len(packet) > 60:
                    data = bytearray(packet)
                    data[-1] ^= 0x01
                    packet = bytes(data)
                out.append(packet)
                if self.rng.random() < self.dup:
                    out.append(packet)
            if held is not None:
                out.append(held)
            return out


def make_pair(net=1):
    a = l3p.Segment(('127.0.0.1', 0))
    b = l3p.Segment(('127.0.0.1', 0))
    a.peers.append(('127.0.0.1', b.port))
    b.peers.append(('127.0.0.1', a.port))
    a.start()
    b.start()
    client = l3p.Node(a, net, 0x0020)
    server = l3p.Node(b, net, 0xFF02)
    return a, b, client, server


class Echo:
    def __init__(self):
        self.seen = []
        self.opened = []
        self.closed = []
        self.lock = threading.Lock()

    def on_open(self, assoc):
        self.opened.append(assoc)

    def on_request(self, assoc, payload):
        with self.lock:
            self.seen.append(bytes(payload))
        return b'R' + bytes(payload)

    def on_close(self, assoc, error):
        self.closed.append((assoc, error))

    def on_ureq(self, src, payload):
        return b'U' + payload


class Base(unittest.TestCase):
    def setUp(self):
        self.seg_a, self.seg_b, self.cnode, self.snode = make_pair()
        self.saved = {name: getattr(l3p, name) for name in
                      ('TXN_GIVEUP', 'STREAM_GIVEUP', 'OPEN_GIVEUP', 'IDLE_DEFAULT', 'CLOSE_GIVEUP')}
        self.endpoints = []

    def tearDown(self):
        for ep in self.endpoints:
            ep.close()
        self.seg_a.close()
        self.seg_b.close()
        for name, value in self.saved.items():
            setattr(l3p, name, value)

    def endpoint(self, node, sock=None, **kw):
        ep = l3p.Endpoint(node, sock, **kw)
        self.endpoints.append(ep)
        return ep

    def server(self, kind=l3p.KIND_TXN, server=None, **kw):
        ep = self.endpoint(self.snode, 9, **kw)
        server = server or Echo()
        ep.listen(kind, server)
        return ep, server


class TransactionTests(Base):
    def test_call_and_close(self):
        _, echo = self.server()
        client = self.endpoint(self.cnode)
        assoc = client.open_txn(1, 0xFF02, 9)
        self.assertEqual(assoc.call(b'hello'), b'Rhello')
        self.assertEqual(assoc.call(b''), b'R')
        assoc.close()
        self.assertEqual(assoc.state, 'closed')
        time.sleep(0.2)
        self.assertEqual(len(echo.closed), 1)
        self.assertIsNone(echo.closed[0][1])

    def test_net_zero_reaches_the_local_network(self):
        self.server()
        client = self.endpoint(self.cnode)
        assoc = client.open_txn(0, 0xFF02, 9)
        self.assertEqual(assoc.call(b'x'), b'Rx')

    def test_pipelined_requests_keep_order(self):
        _, echo = self.server()
        client = self.endpoint(self.cnode)
        assoc = client.open_txn(1, 0xFF02, 9)
        xids = [assoc.send(bytes([i])) for i in range(4)]
        self.assertEqual([assoc.wait(x) for x in xids], [b'R' + bytes([i]) for i in range(4)])
        self.assertEqual(echo.seen, [bytes([i]) for i in range(4)])

    def test_exactly_once_under_loss_duplication_and_reordering(self):
        _, echo = self.server()
        self.seg_a.mangle = Lossy(1, drop=0.2, dup=0.1, reorder=0.1)
        self.seg_b.mangle = Lossy(2, drop=0.2, dup=0.1, reorder=0.1)
        client = self.endpoint(self.cnode)
        assoc = client.open_txn(1, 0xFF02, 9)
        count = 1000
        pending = []
        for i in range(count):
            pending.append((i, assoc.send(struct.pack('>I', i))))
            while len(pending) >= 4 or (i == count - 1 and pending):
                j, xid = pending.pop(0)
                self.assertEqual(assoc.wait(xid), b'R' + struct.pack('>I', j))
        self.assertEqual(echo.seen, [struct.pack('>I', i) for i in range(count)])

    def test_open_retransmission_creates_one_association(self):
        self.seg_b.mangle = Lossy(3, drop=0.5)
        _, echo = self.server()
        client = self.endpoint(self.cnode)
        assoc = client.open_txn(1, 0xFF02, 9)
        assoc.call(b'x')
        time.sleep(0.3)
        self.assertEqual(len(echo.opened), 1)

    def test_reset_reasons(self):
        client = self.endpoint(self.cnode)
        with self.assertRaises(l3p.L3PError) as cm:
            client.open_txn(1, 0xFF02, 9, timeout=5)
        self.assertEqual(cm.exception.reason, 0)  # nobody bound: open times out
        self.assertEqual(cm.exception.code, 'TIMEOUT')

    def test_no_listener_refused_busy_enc_required(self):
        ep = self.endpoint(self.snode, 9)
        client = self.endpoint(self.cnode)
        with self.assertRaises(l3p.L3PError) as cm:
            client.open_txn(1, 0xFF02, 9)
        self.assertEqual(cm.exception.reason, l3p.R_NO_LISTENER)
        ep.listen(l3p.KIND_TXN, Echo())
        ep.allow = lambda peer: False
        with self.assertRaises(l3p.L3PError) as cm:
            client.open_txn(1, 0xFF02, 9)
        self.assertEqual(cm.exception.reason, l3p.R_REFUSED)
        ep.allow = None
        ep.max_assocs = 1
        client.open_txn(1, 0xFF02, 9)
        with self.assertRaises(l3p.L3PError) as cm:
            client.open_txn(1, 0xFF02, 9)
        self.assertEqual(cm.exception.reason, l3p.R_BUSY)
        ep.max_assocs = 8
        ep.require_encryption = True
        with self.assertRaises(l3p.L3PError) as cm:
            client.open_txn(1, 0xFF02, 9)
        self.assertEqual(cm.exception.reason, l3p.R_ENC_REQUIRED)

    def test_timeout_breaks_the_association_then_stale(self):
        l3p.TXN_GIVEUP = 1.0
        self.server()
        client = self.endpoint(self.cnode)
        assoc = client.open_txn(1, 0xFF02, 9)
        self.seg_a.mangle = lambda packet: []
        with self.assertRaises(l3p.L3PError) as cm:
            assoc.call(b'x')
        self.assertEqual(cm.exception.code, 'TIMEOUT')
        with self.assertRaises(l3p.L3PError) as cm:
            assoc.call(b'y')
        self.assertEqual(cm.exception.code, 'STALE')

    def test_unknown_conn_gets_reset(self):
        ep, echo = self.server()
        client = self.endpoint(self.cnode)
        assoc = client.open_txn(1, 0xFF02, 9)
        with ep.lock:
            ep.assocs.clear()  # the server forgets the association
        with self.assertRaises(l3p.L3PError) as cm:
            assoc.call(b'x')
        self.assertEqual(cm.exception.reason, l3p.R_UNKNOWN_CONN)

    def test_idle_timeout_and_keepalive(self):
        l3p.IDLE_DEFAULT = 1
        ep, echo = self.server(idle=1)
        client = self.endpoint(self.cnode)
        assoc = client.open_txn(1, 0xFF02, 9)
        time.sleep(2.0)  # keepalives every 0.25 s keep it open
        self.assertEqual(assoc.call(b'x'), b'Rx')
        self.seg_a.mangle = lambda packet: []
        time.sleep(1.6)
        self.assertEqual(ep.count(), 0)

    def test_window_outside_and_old_xids_are_dropped(self):
        ep, echo = self.server()
        client = self.endpoint(self.cnode)
        assoc = client.open_txn(1, 0xFF02, 9)
        assoc.call(b'a')
        header = l3p.HEADER.pack(0x11, l3p.T_REQ, 0, 0, assoc.conn, 50, 2, 0, 0)
        self.cnode.send_datagram(1, 0xFF02, 9, client.socket, l3p.PROTO_L3P, header + b'far')
        header = l3p.HEADER.pack(0x11, l3p.T_REQ, 0, 0, assoc.conn, 1, 2, 0, 0)
        self.cnode.send_datagram(1, 0xFF02, 9, client.socket, l3p.PROTO_L3P, header + b'old')
        time.sleep(0.3)
        self.assertEqual(assoc.call(b'b'), b'Rb')
        self.assertEqual(echo.seen, [b'a', b'b'])


class ConnectionlessTests(Base):
    def test_unicast_and_broadcast(self):
        self.server()
        other_seg = l3p.Segment(('127.0.0.1', 0))
        other_seg.peers.append(('127.0.0.1', self.seg_a.port))
        self.seg_a.peers.append(('127.0.0.1', other_seg.port))
        other_seg.start()
        try:
            other = l3p.Node(other_seg, 1, 0xFF03)
            ep2 = self.endpoint(other, 9)
            ep2.listen(l3p.KIND_TXN, Echo())
            client = self.endpoint(self.cnode)
            answers = client.ucall(1, 0xFF02, 9, b'q')
            self.assertEqual(answers, [((1, 0xFF02, 9), b'Uq')])
            answers = client.ucall(1, l3p.BROADCAST, 9, b'b', wait=0.5)
            self.assertEqual(sorted(src[1] for src, _ in answers), [0xFF02, 0xFF03])
        finally:
            other_seg.close()


class StreamServer:
    def __init__(self):
        self.streams = []

    def on_open(self, stream):
        self.streams.append(stream)
        threading.Thread(target=self.echo, args=(stream,), daemon=True).start()

    def echo(self, stream):
        try:
            while True:
                data = stream.read()
                if not data:
                    break
                stream.write(data)
            stream.close_write()
        except l3p.L3PError:
            pass


class StreamTests(Base):
    def bulk(self, size, server_kw=None, **kw):
        self.server(l3p.KIND_STREAM, StreamServer(), **(server_kw or {}))
        client = self.endpoint(self.cnode)
        stream = client.open_stream(1, 0xFF02, 9, **kw)
        data = random.Random(5).randbytes(size)
        received = bytearray()

        def reader():
            while True:
                chunk = stream.read(timeout=120)
                if not chunk:
                    return
                received.extend(chunk)

        thread = threading.Thread(target=reader)
        thread.start()
        stream.write(data)
        stream.close_write()
        thread.join(180)
        self.assertEqual(hashlib.sha256(received).digest(), hashlib.sha256(data).digest())
        stream.close()
        self.assertEqual(stream.state, 'closed')
        return stream

    def test_bulk_one_mebibyte_with_loss(self):
        self.seg_a.mangle = Lossy(7, drop=0.2, dup=0.05, reorder=0.05)
        self.seg_b.mangle = Lossy(8, drop=0.2, dup=0.05, reorder=0.05)
        self.bulk(1 << 20)

    def test_inflight_is_limited_to_four_packets(self):
        sent = []

        def watch(packet):
            frame = packet[12:]
            parsed = l3p.parse_datagram(frame[8:])
            if parsed and parsed[6] == l3p.PROTO_L3P and parsed[7][1] == l3p.T_DATA:
                sent.append(len(parsed[7]))
            return [packet]

        self.server(l3p.KIND_STREAM, StreamServer())
        client = self.endpoint(self.cnode)
        stream = client.open_stream(1, 0xFF02, 9)
        self.seg_b.mangle = lambda packet: []  # no ACKs come back
        self.seg_a.mangle = watch
        stream.write(b'x' * 20000)
        time.sleep(0.2)
        self.assertEqual(len(sent), 4)
        stream.abort()

    def test_zero_window_recovery(self):
        ep, server = self.server(l3p.KIND_STREAM, type('S', (), {'on_open': lambda self, s: None})())
        client = self.endpoint(self.cnode)
        stream = client.open_stream(1, 0xFF02, 9)
        stream.write(b'y' * (l3p.STREAM_RCVBUF + 5000))  # the server reads nothing yet
        time.sleep(1.0)
        self.assertEqual(stream.peer_win, 0)
        with ep.lock:
            peer = next(iter(ep.assocs.values()))
        got = 0
        while got < l3p.STREAM_RCVBUF + 5000:
            got += len(peer.read(timeout=10))
        self.assertEqual(got, l3p.STREAM_RCVBUF + 5000)

    def test_half_close_and_abort(self):
        ep, server = self.server(l3p.KIND_STREAM, StreamServer())
        client = self.endpoint(self.cnode)
        stream = client.open_stream(1, 0xFF02, 9)
        stream.write(b'ping')
        stream.close_write()
        self.assertEqual(stream.read(timeout=5), b'ping')
        self.assertEqual(stream.read(timeout=5), b'')
        stream2 = client.open_stream(1, 0xFF02, 9)
        stream2.write(b'a')
        self.assertEqual(stream2.read(timeout=5), b'a')
        stream2.abort('bye')
        time.sleep(0.3)
        self.assertEqual(server.streams[-1].state, 'broken')
        self.assertEqual(server.streams[-1].error.reason, l3p.R_ABORT)


@unittest.skipUnless(l3p.HAVE_CRYPTO, 'needs the cryptography package')
class EncryptionTests(Base):
    def setUp(self):
        super().setUp()
        self.server_key = l3p.x25519_private()
        self.client_key = l3p.x25519_private()

    def test_handshake_agrees_on_keys(self):
        for psk in (None, bytes(range(32))):
            i = l3p.Handshake(True, self.client_key, b'p', psk)
            r = l3p.Handshake(False, self.server_key, b'p', psk)
            r.read(i.write())
            i.read(r.write())
            r.read(i.write())
            self.assertEqual(i.split(), r.split())
            self.assertEqual(i.remote_static, l3p.public_key(self.server_key))
            self.assertEqual(r.remote_static, l3p.public_key(self.client_key))
        i = l3p.Handshake(True, self.client_key, b'p')
        r = l3p.Handshake(False, self.server_key, b'q')  # different prologue
        r.read(i.write())
        with self.assertRaises(l3p.InvalidTag):
            i.read(r.write())

    def test_encrypted_transactions_under_loss_and_tampering(self):
        _, echo = self.server(key=self.server_key)
        self.seg_a.mangle = Lossy(11, drop=0.15, dup=0.1, reorder=0.1, corrupt=0.1)
        self.seg_b.mangle = Lossy(12, drop=0.15, dup=0.1, reorder=0.1, corrupt=0.1)
        client = self.endpoint(self.cnode, key=self.client_key)
        seen = []
        assoc = client.open_txn(1, 0xFF02, 9, encrypt=True, verify=lambda k: seen.append(k) or True)
        self.assertEqual(seen, [l3p.public_key(self.server_key)])
        self.assertTrue(assoc.encrypted)
        self.assertEqual(assoc.mtu(), l3p.MAX_ENC)
        for i in range(200):
            self.assertEqual(assoc.call(struct.pack('>I', i)), b'R' + struct.pack('>I', i))
        self.assertEqual(echo.seen, [struct.pack('>I', i) for i in range(200)])
        self.assertEqual(echo.opened[0].peer_key, l3p.public_key(self.client_key))
        assoc.close()

    def test_plaintext_reset_is_ignored_after_handshake(self):
        _, echo = self.server(key=self.server_key)
        client = self.endpoint(self.cnode, key=self.client_key)
        assoc = client.open_txn(1, 0xFF02, 9, encrypt=True)
        header = l3p.HEADER.pack(0x11, l3p.T_RESET, 0, 0, assoc.conn, 0, 0, 0, 0)
        self.snode.send_datagram(1, 0x0020, client.socket, 9, l3p.PROTO_L3P, header + l3p.reset_payload(9))
        time.sleep(0.2)
        self.assertEqual(assoc.call(b'x'), b'Rx')

    def test_encrypted_stream(self):
        StreamTests.bulk(self, 200000, {'key': self.server_key}, encrypt=True)

    def test_key_checks(self):
        ep, _ = self.server(key=self.server_key, allow_keys=[l3p.fingerprint(l3p.public_key(self.client_key))])
        stranger = self.endpoint(self.cnode, key=l3p.x25519_private())
        with self.assertRaises(l3p.L3PError) as cm:
            stranger.open_txn(1, 0xFF02, 9, encrypt=True)
        self.assertEqual(cm.exception.reason, l3p.R_AUTH_FAILED)
        with self.assertRaises(l3p.L3PError) as cm:
            stranger.open_txn(1, 0xFF02, 9)
        self.assertEqual(cm.exception.reason, l3p.R_ENC_REQUIRED)
        client = self.endpoint(self.cnode, key=self.client_key)
        with self.assertRaises(l3p.L3PError) as cm:
            client.open_txn(1, 0xFF02, 9, encrypt=True, verify=lambda key: False)
        self.assertEqual(cm.exception.reason, l3p.R_AUTH_FAILED)
        assoc = client.open_txn(1, 0xFF02, 9, encrypt=True)
        self.assertEqual(assoc.call(b'ok'), b'Rok')

    def test_psk(self):
        psk = bytes(range(32))
        self.server(key=self.server_key, psk=psk)
        good = self.endpoint(self.cnode, key=self.client_key, psk=psk)
        self.assertEqual(good.open_txn(1, 0xFF02, 9, encrypt=True).call(b'p'), b'Rp')
        bad = self.endpoint(self.cnode, key=self.client_key, psk=bytes(32))
        with self.assertRaises(l3p.L3PError):
            bad.open_txn(1, 0xFF02, 9, encrypt=True, timeout=3)
        none = self.endpoint(self.cnode, key=self.client_key)
        with self.assertRaises(l3p.L3PError) as cm:
            none.open_txn(1, 0xFF02, 9, encrypt=True)
        self.assertEqual(cm.exception.reason, l3p.R_AUTH_FAILED)

    def test_known_hosts(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            hosts = l3p.KnownHosts(Path(tmp) / 'known_hosts')
            self.assertIsNone(hosts.lookup('share'))
            hosts.add('share', b'\x01' * 32)
            self.assertEqual(hosts.lookup('share'), b'\x01' * 32)
            key = l3p.load_or_create_key(Path(tmp) / 'key')
            self.assertEqual(l3p.private_bytes(l3p.load_or_create_key(Path(tmp) / 'key')), l3p.private_bytes(key))


if __name__ == '__main__':
    unittest.main()
