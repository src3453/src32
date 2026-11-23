#!/usr/bin/env python3
"""L3P (Layer 3 Protocol, doc/spec_NET_L3P.md) and the CNET UDP node.

Host programs on a CNET segment (the TCP bridge, the CFP server, the
store-and-forward relay) share this module. It holds

* the CNET wire format and the UDP physical layer (doc/spec_NET.md §3, §4,
  §8.3): ``Segment`` sends and receives link frames, ``Node`` answers PROBE
  and echo and dispatches datagrams by socket;
* L3P on top of it (PROTO 0x04): ``Endpoint`` is one CNET socket that
  multiplexes associations by CONN. Transactions (``TxnAssoc``, §5),
  streams (``StreamAssoc``, §6), connectionless queries (UREQ/URSP, §5.5)
  and, when the ``cryptography`` package is present, the optional
  Noise XX encryption (§7).

Retransmission and timeouts run in real time on one timer thread per
endpoint. Upper-layer callbacks (accepting associations, serving requests,
answering UREQ) run on one worker thread per endpoint, in arrival order.
"""
import hashlib
import hmac
import os
from pathlib import Path
import queue
import random
import socket
import struct
import sys
import threading
import time

# ---- CNET wire format (doc/spec_NET.md §3, §4, §8.3) ----

LINK_DATAGRAM = 0x01
LINK_PROBE = 0x81
LINK_IN_USE = 0x82
BROADCAST = 0xFFFF
ECHO_SOCKET = 4
PROTO_ECHO = 0x02
MTU = 1024
UDP_MAGIC = b'CNET'
DYNAMIC_SOCKETS = range(128, 255)


def crc16(data, crc=0xFFFF):
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) if crc & 0x8000 else crc << 1
            crc &= 0xFFFF
    return crc


def datagram(dst_net, dst_node, src_net, src_node, dst_socket, src_socket, proto, payload):
    header = bytearray(struct.pack('>BBHHHHHHBB', 0x10, proto, len(payload), 0, dst_net, dst_node,
                                   src_net, src_node, dst_socket, src_socket))
    struct.pack_into('>H', header, 4, crc16(bytes(header) + payload))
    return bytes(header) + payload


def parse_datagram(payload):
    """Returns (dst_net, dst_node, src_net, src_node, dst_socket, src_socket, proto, body)
    or None when the version, length or CRC is wrong."""
    if len(payload) < 16:
        return None
    ver_hop, proto, length, crc, dnet, dnode, snet, snode, dsock, ssock = struct.unpack(
        '>BBHHHHHHBB', payload[:16])
    body = payload[16:]
    check = bytearray(payload[:16])
    check[0] &= 0xF0
    check[4:6] = b'\0\0'
    if ver_hop >> 4 != 1 or length != len(body) or crc16(bytes(check) + body) != crc:
        return None
    return dnet, dnode, snet, snode, dsock, ssock, proto, bytes(body)


def link_frame(dst, src, kind, payload):
    return struct.pack('>HHBBH', dst, src, kind, 0, len(payload)) + payload


def parse_hostport(text, default_port=None):
    host, sep, port = text.rpartition(':')
    if not sep:
        if default_port is None:
            raise ValueError(f'{text}: expected HOST:PORT')
        return text, default_port
    return host.strip('[]') or '0.0.0.0', int(port)


def _number(text, base, limit):
    digits = '0123456789ABCDEF'[:base]
    if not text or any(c not in digits for c in text.upper()):
        raise ValueError(f'bad number {text!r}')
    value = int(text, base)
    if value > limit:
        raise ValueError(f'{text}: out of range')
    return value


def split_notation(text):
    """(body, base) for the spec_NET §2 notation: decimal by default, all
    hexadecimal when the whole string ends in H/h. The two never mix."""
    if text[-1:] in ('H', 'h'):
        return text[:-1], 16
    return text, 10


def parse_node(text):
    """A bare node number: decimal, or hex with a trailing H."""
    body, base = split_notation(text)
    return _number(body, base, 0xFFFF)


def parse_endpoint(text):
    """CNET NET.NODE[:SOCK] (spec_NET §2). NET and NODE are 0-65535, SOCK
    0-255 (decimal, or all hex with a trailing H). .NODE and NODE mean NET 0.
    Returns (net, node, sock or None)."""
    try:
        body, base = split_notation(text)
        addr, sep, sock = body.partition(':')
        net, dot, node = addr.rpartition('.')
        if not dot:
            net, node = '', addr
        return (_number(net, base, 0xFFFF) if net else 0, _number(node, base, 0xFFFF),
                _number(sock, base, 0xFF) if sep else None)
    except ValueError:
        raise ValueError(f'{text}: bad CNET address') from None


def parse_addr(text):
    """CNET address in the spec_NET §2 notation: NET.NODE, .NODE or NODE."""
    net, node, sock = parse_endpoint(text)
    if sock is not None or not 0 <= net <= 0xFFFE or not 1 <= node <= 0xFFFF:
        raise ValueError(f'{text}: bad CNET address')
    return net, node


def format_addr(net, node):
    return f'{net}.{node}'


def log(*args):
    print(time.strftime('%H:%M:%S'), *args, file=sys.stderr, flush=True)


class Segment:
    """One UDP segment: sends link frames to the peers (and the multicast
    group) and hands received ones to on_frame. Unicast senders of valid
    packets are learned as peers when no multicast group is used, so an
    emulator only has to list this program in its udp.peers.

    ``mangle``, when set, is called with each outgoing packet and returns the
    packets to really send (tests use it to drop, duplicate and reorder)."""

    def __init__(self, bind, segment_id=0, multicast=None, peers=(), learn=True):
        self.segment_id = segment_id
        self.sender = random.getrandbits(32)
        self.peers = list(peers)
        self.multicast = multicast
        self.learn = learn and not multicast
        self.learned = {}
        self.on_frame = None
        self.mangle = None
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        host, port = bind
        if multicast:
            self.sock.bind(('0.0.0.0' if host in ('', '0.0.0.0') else host, port))
            group = socket.inet_aton(multicast[0])
            self.sock.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP, group + socket.inet_aton('0.0.0.0'))
            self.sock.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_LOOP, 1)
        else:
            self.sock.bind((host, port))
        self.sock.settimeout(0.2)
        self.port = self.sock.getsockname()[1]
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.run, name='cnet-udp', daemon=True)

    def start(self):
        self.thread.start()

    def close(self):
        self.stop.set()
        if self.thread.is_alive():
            self.thread.join(timeout=2)
        self.sock.close()

    def send(self, frame):
        packet = UDP_MAGIC + struct.pack('>BBHI', 1, 0, self.segment_id, self.sender) + frame
        packets = self.mangle(packet) if self.mangle else (packet,)
        targets = list(self.peers)
        if self.multicast:
            targets.append(self.multicast)
        now = time.monotonic()
        targets += [a for a, seen in list(self.learned.items()) if now - seen < 120 and a not in targets]
        for packet in packets:
            for target in targets:
                try:
                    self.sock.sendto(packet, target)
                except OSError:
                    pass

    def run(self):
        while not self.stop.is_set():
            try:
                packet, addr = self.sock.recvfrom(4096)
            except socket.timeout:
                continue
            except OSError:  # e.g. WSAECONNRESET after an ICMP port unreachable
                if self.stop.is_set():
                    return
                continue
            if len(packet) < 20 or packet[:4] != UDP_MAGIC or packet[4] != 1:
                continue
            segment_id, sender = struct.unpack('>HI', packet[6:12])
            if segment_id != self.segment_id or sender == self.sender:
                continue
            frame = packet[12:]
            if struct.unpack('>H', frame[6:8])[0] != len(frame) - 8:
                continue
            if self.learn and addr not in self.peers:
                if addr in self.learned or len(self.learned) < 32:
                    self.learned[addr] = time.monotonic()
            if self.on_frame:
                try:
                    self.on_frame(frame)
                except Exception as error:  # a bad frame must not stop the node
                    log('frame error:', repr(error))


class Node:
    """The link layer of one CNET node: answers PROBE for its own number and
    echo requests, and hands other datagrams to ``handlers[socket]`` as
    fn(src_net, src_node, src_socket, proto, body, dst_net, dst_node)."""

    def __init__(self, segment, net=0, node=0xFF01):
        self.segment = segment
        self.net = net
        self.node = node
        self.handlers = {}
        self.lock = threading.Lock()
        segment.on_frame = self.on_frame

    def alloc_socket(self):
        with self.lock:
            free = [s for s in DYNAMIC_SOCKETS if s not in self.handlers]
            if not free:
                raise OSError('no free CNET socket')
            sock = random.choice(free)
            self.handlers[sock] = None
            return sock

    def send_datagram(self, dst_net, dst_node, dst_socket, src_socket, proto, payload):
        body = datagram(dst_net, dst_node, self.net, self.node, dst_socket, src_socket, proto, payload)
        self.segment.send(link_frame(dst_node, self.node, LINK_DATAGRAM, body))

    def on_frame(self, frame):
        dst, src, kind, _, _ = struct.unpack('>HHBBH', frame[:8])
        payload = frame[8:]
        if kind == LINK_PROBE:
            if dst == self.node:
                self.segment.send(link_frame(self.node, self.node, LINK_IN_USE, b''))
            return
        if kind != LINK_DATAGRAM or dst not in (self.node, BROADCAST):
            return
        parsed = parse_datagram(payload)
        if not parsed:
            return
        dnet, dnode, snet, snode, dsock, ssock, proto, body = parsed
        if dnet not in (0, self.net) or dnode not in (self.node, BROADCAST):
            return
        if dsock == ECHO_SOCKET:
            if proto == PROTO_ECHO and body[:1] == b'\x01':
                self.send_datagram(snet, snode, ssock, ECHO_SOCKET, PROTO_ECHO, b'\x02' + body[1:])
            return
        handler = self.handlers.get(dsock)
        if handler:
            handler(snet, snode, ssock, proto, body, dnet, dnode)


# ---- L3P (doc/spec_NET_L3P.md) ----

PROTO_L3P = 0x04
L3P_VERSION = 1
KIND_TXN, KIND_STREAM = 1, 2
T_OPEN, T_ACCEPT, T_RESET, T_CLOSE, T_CLOSED, T_ACKONLY, T_HS, T_READY = range(1, 9)
T_REQ, T_RSP, T_UREQ, T_URSP, T_DATA = 0x10, 0x11, 0x18, 0x19, 0x20
F_ENC, F_FIN = 0x01, 0x02
OPT_ENC, OPT_PSK = 0x01, 0x02
HEADER = struct.Struct('>BBBBIIIHH')
MAX_PLAIN = MTU - HEADER.size         # 1004
MAX_ENC = MAX_PLAIN - 4 - 16          # 984
TAG = 16

R_NO_LISTENER, R_BUSY, R_REFUSED, R_ENC_REQUIRED, R_ENC_UNSUPPORTED = 1, 2, 3, 4, 5
R_AUTH_FAILED, R_UNKNOWN_CONN, R_TIMEOUT, R_ABORT = 6, 7, 8, 9
REASONS = {1: 'NO_LISTENER', 2: 'BUSY', 3: 'REFUSED', 4: 'ENC_REQUIRED', 5: 'ENC_UNSUPPORTED',
           6: 'AUTH_FAILED', 7: 'UNKNOWN_CONN', 8: 'TIMEOUT', 9: 'ABORT'}

RTO_FIRST = 0.3
RTO_MAX = 2.4
TXN_GIVEUP = 15.0
STREAM_GIVEUP = 60.0
OPEN_GIVEUP = 15.0
CLOSE_GIVEUP = 15.0
IDLE_DEFAULT = 120
TXN_WINDOW_MAX = 4
STREAM_INFLIGHT = 4                   # unacknowledged DATA packets (card queue)
STREAM_RCVBUF = 16 * 1024
STREAM_SNDBUF = 64 * 1024
MASK = 0xFFFFFFFF
PN_LIMIT = 0x7FFFFFFF
TICK = 0.02

VALID_TYPES = {
    KIND_TXN: {T_OPEN, T_ACCEPT, T_RESET, T_CLOSE, T_CLOSED, T_ACKONLY, T_HS, T_READY,
               T_REQ, T_RSP, T_UREQ, T_URSP},
    KIND_STREAM: {T_OPEN, T_ACCEPT, T_RESET, T_CLOSE, T_CLOSED, T_ACKONLY, T_HS, T_READY, T_DATA},
}


class L3PError(Exception):
    """An association failed. ``code`` is 'TIMEOUT' (gave up retransmitting),
    'STALE' (the association was already lost), 'RESET' (the peer refused or
    aborted; ``reason`` is the RESET REASON) or 'CLOSED'."""

    def __init__(self, code, reason=0, message=''):
        text = code
        if reason:
            text += f' {REASONS.get(reason, reason)}'
        if message:
            text += f': {message}'
        super().__init__(text)
        self.code = code
        self.reason = reason
        self.message = message


def reset_payload(reason, message=''):
    return struct.pack('>HH', reason, 0) + message.encode('utf-8')[:128]


def parse_reset(payload):
    if len(payload) < 4:
        return 0, ''
    reason, _ = struct.unpack('>HH', payload[:4])
    return reason, payload[4:4 + 128].decode('utf-8', 'replace')


def _same_peer(a, b):
    """Endpoints match when node and socket agree and the networks agree or
    one of them is 0 (this network)."""
    return a[1] == b[1] and a[2] == b[2] and (a[0] == b[0] or a[0] == 0 or b[0] == 0)


# ---- Noise_XX_25519_ChaChaPoly_SHA256 (§7) ----

try:
    from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey, X25519PublicKey
    from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
    from cryptography.hazmat.primitives import serialization
    from cryptography.exceptions import InvalidTag
    HAVE_CRYPTO = True
except ImportError:  # encryption is optional (§1.3)
    HAVE_CRYPTO = False

    class InvalidTag(Exception):
        pass


def _raw_public(private):
    return private.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


def x25519_private(raw=None):
    """An X25519 key from 32 raw bytes, or a fresh one."""
    if raw is None:
        return X25519PrivateKey.generate()
    return X25519PrivateKey.from_private_bytes(raw)


def private_bytes(private):
    return private.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw,
                                 serialization.NoEncryption())


def public_key(private):
    return _raw_public(private)


def fingerprint(public):
    """SHA-256 of the public key, first 16 bytes in hex (§7.4)."""
    return hashlib.sha256(public).digest()[:16].hex()


def _hmac(key, data):
    return hmac.new(key, data, hashlib.sha256).digest()


def _hkdf(ck, ikm, count):
    temp = _hmac(ck, ikm)
    out1 = _hmac(temp, b'\x01')
    out2 = _hmac(temp, out1 + b'\x02')
    if count == 2:
        return out1, out2
    return out1, out2, _hmac(temp, out2 + b'\x03')


class _Cipher:
    def __init__(self):
        self.k = None
        self.n = 0

    def set(self, key):
        self.k = key
        self.n = 0

    def encrypt(self, ad, data):
        if self.k is None:
            return data
        nonce = b'\0' * 4 + self.n.to_bytes(8, 'little')
        self.n += 1
        return ChaCha20Poly1305(self.k).encrypt(nonce, data, ad)

    def decrypt(self, ad, data):
        if self.k is None:
            return data
        nonce = b'\0' * 4 + self.n.to_bytes(8, 'little')
        out = ChaCha20Poly1305(self.k).decrypt(nonce, data, ad)
        self.n += 1
        return out


class Handshake:
    """One side of Noise_XX (or XXpsk3) with empty payloads. The initiator
    calls write(), read(), write(); the responder read(), write(), read().
    After the last message ``split()`` returns (initiator->responder key,
    responder->initiator key) and ``remote_static`` is the peer's key."""

    def __init__(self, initiator, static, prologue, psk=None):
        name = b'Noise_XXpsk3_25519_ChaChaPoly_SHA256' if psk else b'Noise_XX_25519_ChaChaPoly_SHA256'
        self.h = name.ljust(32, b'\0') if len(name) <= 32 else hashlib.sha256(name).digest()
        self.ck = self.h
        self.cipher = _Cipher()
        self.initiator = initiator
        self.s = static
        self.e = None
        self.re = None
        self.remote_static = None
        self.psk = psk
        self.step = 0
        self._mix_hash(prologue)

    def _mix_hash(self, data):
        self.h = hashlib.sha256(self.h + data).digest()

    def _mix_key(self, ikm):
        self.ck, key = _hkdf(self.ck, ikm, 2)
        self.cipher.set(key)

    def _mix_key_and_hash(self, ikm):
        self.ck, temp_h, key = _hkdf(self.ck, ikm, 3)
        self._mix_hash(temp_h)
        self.cipher.set(key)

    def _encrypt_hash(self, data):
        out = self.cipher.encrypt(self.h, data)
        self._mix_hash(out)
        return out

    def _decrypt_hash(self, data):
        out = self.cipher.decrypt(self.h, data)
        self._mix_hash(data)
        return out

    @staticmethod
    def _dh(private, public):
        return private.exchange(X25519PublicKey.from_public_bytes(public))

    def _tokens(self):
        return (('e',), ('e', 'ee', 's', 'es'), ('s', 'se') + (('psk',) if self.psk else ()))[self.step]

    def write(self, payload=b''):
        out = bytearray()
        for token in self._tokens():
            if token == 'e':
                self.e = X25519PrivateKey.generate()
                pub = _raw_public(self.e)
                out += pub
                self._mix_hash(pub)
                if self.psk:
                    self._mix_key(pub)
            elif token == 's':
                out += self._encrypt_hash(_raw_public(self.s))
            else:
                self._token_dh(token)
        out += self._encrypt_hash(payload)
        self.step += 1
        return bytes(out)

    def read(self, message):
        """Returns the payload; raises ValueError or InvalidTag on a bad message."""
        message = bytes(message)
        pos = 0
        for token in self._tokens():
            if token == 'e':
                if len(message) < pos + 32:
                    raise ValueError('short handshake message')
                self.re = message[pos:pos + 32]
                pos += 32
                self._mix_hash(self.re)
                if self.psk:
                    self._mix_key(self.re)
            elif token == 's':
                size = 32 + (TAG if self.cipher.k is not None else 0)
                if len(message) < pos + size:
                    raise ValueError('short handshake message')
                self.remote_static = self._decrypt_hash(message[pos:pos + size])
                pos += size
            else:
                self._token_dh(token)
        payload = self._decrypt_hash(message[pos:])
        self.step += 1
        return payload

    def _token_dh(self, token):
        if token == 'ee':
            self._mix_key(self._dh(self.e, self.re))
        elif token == 'es':
            self._mix_key(self._dh(self.e, self.remote_static) if self.initiator else self._dh(self.s, self.re))
        elif token == 'se':
            self._mix_key(self._dh(self.s, self.re) if self.initiator else self._dh(self.e, self.remote_static))
        elif token == 'psk':
            self._mix_key_and_hash(self.psk)

    def split(self):
        return _hkdf(self.ck, b'', 2)


def default_key_dir():
    return Path.home() / '.cpt32' / 'l3p'


def load_or_create_key(path=None):
    """The node's static X25519 key (32 raw bytes in ``path``), created on
    first use (§7.4)."""
    path = Path(path) if path else default_key_dir() / 'key'
    if path.is_file():
        raw = path.read_bytes()
        if len(raw) != 32:
            raise ValueError(f'{path}: expected a 32-byte X25519 key')
        return x25519_private(raw)
    key = x25519_private()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(private_bytes(key))
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass
    return key


class KnownHosts:
    """Service name -> static public key, one ``NAME HEXKEY`` per line (§7.4)."""

    def __init__(self, path=None):
        self.path = Path(path) if path else default_key_dir() / 'known_hosts'

    def lookup(self, name):
        if not self.path.is_file():
            return None
        for line in self.path.read_text(encoding='ascii', errors='replace').splitlines():
            parts = line.split()
            if len(parts) == 2 and parts[0] == name:
                try:
                    return bytes.fromhex(parts[1])
                except ValueError:
                    return None
        return None

    def add(self, name, public):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with open(self.path, 'a', encoding='ascii') as f:
            f.write(f'{name} {public.hex()}\n')


# ---- associations ----

class _Retx:
    """A control packet resent at §4.3 intervals until stopped."""

    def __init__(self, build, giveup):
        self.build = build            # fn() -> sends the packet
        now = time.monotonic()
        self.first = now
        self.interval = RTO_FIRST
        self.next = now + RTO_FIRST
        self.giveup = giveup

    def due(self, now):
        if now < self.next:
            return False
        self.interval = min(self.interval * 2, RTO_MAX)
        self.next = now + self.interval
        return True


class Assoc:
    """State shared by both kinds: establishment, encryption, close, idle."""

    def __init__(self, endpoint, kind, peer, initiator):
        self.ep = endpoint
        self.kind = kind
        self.peer = peer              # (net, node, socket) we send to
        self.initiator = initiator
        self.conn = 0
        self.nonce = 0
        self.state = 'opening'        # opening, handshake, open, closing, closed, broken
        self.options = 0
        self.hs = None
        self.tx_key = None
        self.rx_key = None
        self.tx_pn = 0
        self.rx_top = -1
        self.rx_bits = 0
        self.secure = False           # keys in use (handshake finished on our side)
        self.peer_key = None
        self.idle = IDLE_DEFAULT
        now = time.monotonic()
        self.last_rx = now
        self.last_tx = now
        self.cond = threading.Condition(endpoint.lock)
        self.error = None
        self.ctrl = None              # _Retx for OPEN / HS / CLOSE
        self.accept_packet = None     # responder: payload of ACCEPT for repeated OPENs
        self.verify = None
        self.user = None              # free slot for the upper layer

    # -- naming --

    @property
    def name(self):
        net, node, sock = self.peer
        return f'{format_addr(net, node)}:{sock} #{self.conn:08X}'

    @property
    def encrypted(self):
        return bool(self.options & OPT_ENC)

    def mtu(self):
        return MAX_ENC if self.encrypted else MAX_PLAIN

    def info(self):
        return {'peer': self.peer, 'conn': self.conn, 'encrypted': self.encrypted,
                'peer_key': self.peer_key, 'fingerprint': fingerprint(self.peer_key) if self.peer_key else None}

    # -- sending --

    def _send(self, ptype, seq=0, ack=0, window=0, payload=b'', flags=0, conn=None):
        conn = self.conn if conn is None else conn
        encrypt = self.secure and ptype not in (T_OPEN, T_ACCEPT, T_HS)
        if encrypt:
            flags |= F_ENC
        header = HEADER.pack(L3P_VERSION << 4 | self.kind, ptype, flags, 0, conn, seq & MASK, ack & MASK,
                             window & 0xFFFF, 0)
        if encrypt:
            pn = self.tx_pn
            self.tx_pn += 1
            pnb = struct.pack('>I', pn)
            nonce = b'\0' * 8 + pnb
            payload = pnb + ChaCha20Poly1305(self.tx_key).encrypt(nonce, bytes(payload), header + pnb)
        self.last_tx = time.monotonic()
        net, node, sock = self.peer
        self.ep.node.send_datagram(net, node, sock, self.ep.socket, PROTO_L3P, header + bytes(payload))

    def _decrypt(self, header, payload):
        """Plaintext of an ENC packet, or None (bad tag or replayed PN)."""
        if self.rx_key is None or len(payload) < 4 + TAG:
            return None
        pnb = payload[:4]
        pn = struct.unpack('>I', pnb)[0]
        if pn <= self.rx_top - 64:
            return None
        if pn <= self.rx_top and self.rx_bits >> (self.rx_top - pn) & 1:
            return None
        try:
            plain = ChaCha20Poly1305(self.rx_key).decrypt(b'\0' * 8 + pnb, payload[4:], header + pnb)
        except InvalidTag:
            return None
        if pn > self.rx_top:
            shift = pn - self.rx_top
            self.rx_bits = ((self.rx_bits << shift) | 1) & ((1 << 64) - 1) if shift < 64 else 1
            self.rx_top = pn
        else:
            self.rx_bits |= 1 << (self.rx_top - pn)
        return plain

    # -- failure --

    def _fail(self, error, send_reset=None):
        """Marks the association broken (caller holds the lock)."""
        if self.state in ('closed', 'broken'):
            return
        if send_reset is not None:
            self._send(T_RESET, payload=reset_payload(*send_reset))
        self.state = 'broken'
        self.error = error
        self.ctrl = None
        self.ep._forget(self)
        self.cond.notify_all()
        self._on_end(error)

    def _finish(self):
        """Clean end (CLOSE / CLOSED)."""
        if self.state in ('closed', 'broken'):
            return
        self.state = 'closed'
        self.error = L3PError('CLOSED')
        self.ctrl = None
        self.ep._forget(self)
        self.cond.notify_all()
        self._on_end(None)

    def _on_end(self, error):
        if not self.initiator:
            self.ep._event(('close', self, error))

    def _raise(self):
        error = self.error
        if error.code == 'TIMEOUT':
            self.error = L3PError('STALE', message='association lost')
        raise error

    def _wait(self, predicate, timeout):
        deadline = None if timeout is None else time.monotonic() + timeout
        while not predicate():
            if self.error is not None:
                self._raise()
            left = None if deadline is None else deadline - time.monotonic()
            if left is not None and left <= 0:
                raise TimeoutError
            self.cond.wait(0.1 if left is None else min(left, 0.1))

    def wait_open(self, timeout=OPEN_GIVEUP + 1):
        with self.cond:
            self._wait(lambda: self.state == 'open', timeout)

    def abort(self, message='', reason=R_ABORT):
        """Sends RESET (ABORT, or another REASON such as REFUSED) with a
        message for the peer to show, and drops the association."""
        with self.cond:
            if self.state in ('closed', 'broken'):
                return
            self._fail(L3PError('RESET', reason, message or 'aborted'), (reason, message))

    def close(self, timeout=CLOSE_GIVEUP):
        """Sends CLOSE and waits for CLOSED (or RESET, or the give-up time)."""
        with self.cond:
            if self.state in ('closed', 'broken'):
                return
            if self.state != 'open':
                self._fail(L3PError('RESET', R_ABORT, 'closed while opening'), (R_ABORT, ''))
                return
            self.state = 'closing'
            self.ctrl = _Retx(lambda: self._send(T_CLOSE), CLOSE_GIVEUP)
            self._send(T_CLOSE)
            deadline = time.monotonic() + timeout
            while self.state == 'closing' and time.monotonic() < deadline:
                self.cond.wait(0.1)
            if self.state == 'closing':
                self._finish()

    # -- receiving (endpoint lock held) --

    def on_packet(self, ptype, flags, seq, ack, window, payload, header):
        if flags & F_ENC:
            payload = self._decrypt(header, payload)
            if payload is None:
                return
            if self.state == 'handshake' and self.initiator:
                self._handshake_done()
        elif self.secure and self.state != 'handshake' and ptype not in (T_OPEN, T_ACCEPT, T_HS):
            # §7.3: plaintext after the handshake (including RESET) is ignored,
            # except the UNKNOWN_CONN answer to our own repeated CLOSE (§4.4).
            if ptype == T_RESET and self.state == 'closing':
                self._finish()
            return
        self.last_rx = time.monotonic()
        if ptype == T_RESET:
            reason, message = parse_reset(payload)
            if self.state == 'closing':
                self._finish()
            else:
                self._fail(L3PError('RESET', reason, message))
        elif ptype == T_CLOSE:
            self._send(T_CLOSED)
            self._finish()
        elif ptype == T_CLOSED:
            if self.state == 'closing':
                self._finish()
        elif ptype == T_HS:
            self._on_hs(payload)
        elif ptype == T_READY:
            pass  # handled by the decrypt above
        elif self.state in ('open', 'closing'):
            self.on_data(ptype, flags, seq, ack, window, payload)

    def _on_hs(self, payload):
        if self.initiator:
            return
        if self.state == 'open' and self.encrypted:
            self._send(T_READY)
            return
        if self.state != 'handshake':
            return
        try:
            self.hs.read(payload)
        except (ValueError, InvalidTag):
            self._fail(L3PError('RESET', R_AUTH_FAILED, 'bad handshake'), (R_AUTH_FAILED, 'bad handshake'))
            return
        self.peer_key = self.hs.remote_static
        if self.ep.allow_keys is not None and fingerprint(self.peer_key) not in self.ep.allow_keys:
            self._fail(L3PError('RESET', R_AUTH_FAILED, 'key not allowed'), (R_AUTH_FAILED, 'key not allowed'))
            return
        k1, k2 = self.hs.split()
        self.rx_key, self.tx_key = k1, k2
        self.secure = True
        self.hs = None
        self.state = 'open'
        self._send(T_READY)
        self.ep._event(('open', self))

    def _handshake_done(self):
        self.state = 'open'
        self.ctrl = None
        self.cond.notify_all()

    def on_accept(self, payload, window):
        """Initiator: ACCEPT for our OPEN."""
        if self.state != 'opening' or len(payload) < 12:
            return
        nonce, options, idle, _ = struct.unpack('>IIHH', payload[:12])
        self.options = options
        self.idle = idle or IDLE_DEFAULT
        self.peer_window(window)
        if options & OPT_ENC:
            if not self.hs:
                self._fail(L3PError('RESET', R_AUTH_FAILED, 'unexpected encryption'),
                           (R_AUTH_FAILED, 'unexpected encryption'))
                return
            try:
                self.hs.read(payload[12:])
            except (ValueError, InvalidTag):
                self._fail(L3PError('RESET', R_AUTH_FAILED, 'bad handshake'), (R_AUTH_FAILED, 'bad handshake'))
                return
            self.peer_key = self.hs.remote_static
            if self.verify and not self.verify(self.peer_key):
                self._fail(L3PError('RESET', R_AUTH_FAILED, 'server key rejected'),
                           (R_AUTH_FAILED, 'server key rejected'))
                return
            message = self.hs.write()
            self.tx_key, self.rx_key = self.hs.split()
            self.hs = None
            self.state = 'handshake'
            self.ctrl = _Retx(lambda: self._send(T_HS, payload=message), OPEN_GIVEUP)
            self._send(T_HS, payload=message)
            self.secure = True
        else:
            if self.hs:
                self._fail(L3PError('RESET', R_ENC_UNSUPPORTED, 'peer did not accept encryption'),
                           (R_ENC_UNSUPPORTED, ''))
                return
            self.state = 'open'
            self.ctrl = None
            self.cond.notify_all()

    def tick(self, now):
        if self.ctrl:
            if now - self.ctrl.first > self.ctrl.giveup:
                if self.state == 'closing':
                    self._finish()
                else:
                    self._fail(L3PError('TIMEOUT', message='no answer'), (R_TIMEOUT, ''))
                return
            if self.ctrl.due(now):
                self.ctrl.build()
        if self.state not in ('open', 'closing', 'handshake'):
            return
        if now - self.last_rx > self.idle:
            self.state = 'broken'
            self.error = L3PError('TIMEOUT', message='idle timeout')
            self.ep._forget(self)
            self.cond.notify_all()
            self._on_end(self.error)
            return
        if self.state == 'open':
            self.tick_data(now)
            if self.state == 'open' and now - self.last_tx >= self.idle / 4:
                self.keepalive()

    # -- hooks for the kinds --

    def peer_window(self, window):
        pass

    def on_data(self, ptype, flags, seq, ack, window, payload):
        pass

    def tick_data(self, now):
        pass

    def keepalive(self):
        self._send(T_ACKONLY)

    def open_window(self):
        return 0


class TxnAssoc(Assoc):
    """A transaction association (§5). The initiator uses call() or
    send()/wait(); the responder's requests reach the listener's
    on_request(assoc, payload) in XID order."""

    def __init__(self, endpoint, peer, initiator, window=TXN_WINDOW_MAX):
        super().__init__(endpoint, KIND_TXN, peer, initiator)
        self.window = max(1, min(window, TXN_WINDOW_MAX))
        # initiator
        self.next_xid = 1
        self.outstanding = {}         # xid -> [payload, first, next, interval]
        self.results = {}
        # responder
        self.max_ack = 1
        self.next_deliver = 1
        self.held = {}
        self.processing = None
        self.saved = {}

    def open_window(self):
        return self.window

    def peer_window(self, window):
        if self.initiator:
            self.window = max(1, min(window, self.window)) if window else self.window

    def _min_unacked(self):
        return min(self.outstanding) if self.outstanding else self.next_xid

    # -- initiator --

    def send(self, payload, timeout=None):
        payload = bytes(payload)
        with self.cond:
            if len(payload) > self.mtu():
                raise ValueError(f'request of {len(payload)} bytes exceeds {self.mtu()}')
            self._wait(lambda: self.state == 'open' and self.next_xid < self._min_unacked() + self.window, timeout)
            xid = self.next_xid
            self.next_xid += 1
            now = time.monotonic()
            self.outstanding[xid] = [payload, now, now + RTO_FIRST, RTO_FIRST]
            self._send(T_REQ, xid, self._min_unacked(), 0, payload)
            return xid

    def wait(self, xid, timeout=None):
        with self.cond:
            self._wait(lambda: xid in self.results, timeout)
            return self.results.pop(xid)

    def call(self, payload, timeout=None):
        return self.wait(self.send(payload, timeout), timeout)

    def keepalive(self):
        self._send(T_ACKONLY, 0, self._min_unacked() if self.initiator else 0)

    def close(self, timeout=CLOSE_GIVEUP):
        with self.cond:
            try:
                self._wait(lambda: not self.outstanding, TXN_GIVEUP + 1)
            except (L3PError, TimeoutError):
                return
        super().close(timeout)

    def tick_data(self, now):
        if not self.initiator:
            return
        for xid in sorted(self.outstanding):
            entry = self.outstanding[xid]
            if now - entry[1] > TXN_GIVEUP:
                self._fail(L3PError('TIMEOUT', message='request timed out'), (R_TIMEOUT, ''))
                return
            if now >= entry[2]:
                entry[3] = min(entry[3] * 2, RTO_MAX)
                entry[2] = now + entry[3]
                self._send(T_REQ, xid, self._min_unacked(), 0, entry[0])

    def on_data(self, ptype, flags, seq, ack, window, payload):
        if self.initiator:
            if ptype == T_RSP and seq in self.outstanding:
                del self.outstanding[seq]
                self.results[seq] = payload
                self.cond.notify_all()
            return
        if ptype not in (T_REQ, T_ACKONLY):
            return
        if ack > self.max_ack:
            self.max_ack = ack
            for xid in [x for x in self.saved if x < ack]:
                del self.saved[xid]
        if ptype != T_REQ:
            return
        xid = seq
        if xid < self.max_ack:
            return
        if xid in self.saved:
            self._send(T_RSP, xid, 0, 0, self.saved[xid])
            return
        if xid in self.held or xid == self.processing or xid < self.next_deliver:
            return
        if xid >= self.max_ack + self.window:
            return
        self.held[xid] = payload
        self._deliver()

    def _deliver(self):
        if self.processing is None and self.next_deliver in self.held:
            xid = self.next_deliver
            self.processing = xid
            self.ep._event(('req', self, xid, self.held.pop(xid)))

    def reply(self, xid, payload):
        """Responder: the answer to request ``xid`` (called by the worker)."""
        payload = bytes(payload)
        with self.cond:
            if self.processing != xid or self.state not in ('open', 'closing'):
                return
            if len(payload) > self.mtu():
                raise ValueError(f'response of {len(payload)} bytes exceeds {self.mtu()}')
            self.saved[xid] = payload
            self.processing = None
            self.next_deliver += 1
            self._send(T_RSP, xid, 0, 0, payload)
            self._deliver()


class StreamAssoc(Assoc):
    """A stream association (§6): write(), close_write(), read()."""

    def __init__(self, endpoint, peer, initiator):
        super().__init__(endpoint, KIND_STREAM, peer, initiator)
        self.buf = bytearray()        # unacknowledged and unsent bytes from snd_una
        self.snd_una = 0
        self.snd_nxt = 0
        self.inflight = []            # end sequence numbers of DATA in flight
        self.fin_queued = False
        self.fin_sent = False
        self.fin_acked = False
        self.rcv_nxt = 0
        self.rcv_fin = False
        self.rcv_queued = 0
        self.window_was_zero = False
        self.peer_win = STREAM_INFLIGHT * MAX_PLAIN
        self.inbox = []
        self.retries = 0
        self.rto = RTO_FIRST
        self.last_progress = time.monotonic()
        self.next_retx = 0.0

    def open_window(self):
        return self._rcv_window()

    def peer_window(self, window):
        self.peer_win = window

    def _rcv_window(self):
        return max(0, min(0xFFFF, STREAM_RCVBUF - self.rcv_queued))

    # -- upper layer --

    def write(self, data, timeout=None):
        data = bytes(data)
        with self.cond:
            self._wait(lambda: self.state != 'opening' and self.state != 'handshake', timeout)
            while data:
                self._wait(lambda: len(self.buf) < STREAM_SNDBUF, timeout)
                if self.state != 'open':
                    raise self.error or L3PError('CLOSED')
                if self.fin_queued:
                    return
                room = STREAM_SNDBUF - len(self.buf)
                self.buf += data[:room]
                data = data[room:]
                self._pump()

    def close_write(self):
        with self.cond:
            if self.state == 'open' and not self.fin_queued:
                self.fin_queued = True
                self._pump()

    def read(self, timeout=None):
        """Next received bytes; b'' at the end of the stream. Raises L3PError
        when the association fails, TimeoutError after ``timeout``."""
        with self.cond:
            while not self.inbox:
                if self.rcv_fin:
                    return b''
                if self.error is not None:
                    self._raise()
                self._wait(lambda: self.inbox or self.rcv_fin, timeout)
            data = self.inbox.pop(0)
            was_closed = self._rcv_window() < MAX_PLAIN
            self.rcv_queued -= len(data)
            if was_closed and self._rcv_window() >= MAX_PLAIN and self.state == 'open':
                self._send(T_ACKONLY, 0, self.rcv_nxt, self._rcv_window())
            return data

    @property
    def finished(self):
        return self.state in ('closed', 'broken') or (self.fin_acked and self.rcv_fin)

    def close(self, timeout=CLOSE_GIVEUP):
        """Half-closes our side, waits for the peer's FIN and our FIN's ACK
        (up to ``timeout``), then sends CLOSE."""
        self.close_write()
        with self.cond:
            deadline = time.monotonic() + timeout
            while not (self.fin_acked and self.rcv_fin) and self.state == 'open' and time.monotonic() < deadline:
                self.cond.wait(0.1)
        super().close(timeout)

    # -- protocol --

    def _send_data(self, seq, data, flags=0):
        self._send(T_DATA, seq, self.rcv_nxt, self._rcv_window(), data, flags)

    def _pump(self):
        if self.state != 'open':
            return False
        sent = False
        mtu = self.mtu()
        end = self.snd_una + len(self.buf)
        limit = self.snd_una + self.peer_win
        while self.snd_nxt < end and self.snd_nxt < limit and len(self.inflight) < STREAM_INFLIGHT:
            if self.snd_nxt == self.snd_una:
                self.last_progress = time.monotonic()
                self.next_retx = self.last_progress + self.rto
            start = self.snd_nxt - self.snd_una
            n = min(mtu, end - self.snd_nxt, limit - self.snd_nxt)
            self._send_data(self.snd_nxt, bytes(self.buf[start:start + n]))
            self.snd_nxt += n
            self.inflight.append(self.snd_nxt)
            sent = True
        if (self.fin_queued and not self.fin_sent and self.snd_nxt == end
                and len(self.inflight) < STREAM_INFLIGHT):
            if self.snd_nxt == self.snd_una:
                self.last_progress = time.monotonic()
                self.next_retx = self.last_progress + self.rto
            self._send_data(end, b'', F_FIN)
            self.fin_sent = True
            self.snd_nxt = end + 1
            self.inflight.append(self.snd_nxt)
            sent = True
        return sent

    def on_data(self, ptype, flags, seq, ack, window, payload):
        if ptype not in (T_DATA, T_ACKONLY):
            return
        delta = (ack - self.snd_una) & MASK
        if 0 < delta <= self.snd_nxt - self.snd_una:
            del self.buf[:min(delta, len(self.buf))]
            self.snd_una += delta
            self.inflight = [e for e in self.inflight if e > self.snd_una]
            if self.fin_sent and self.snd_una == self.snd_nxt:
                self.fin_acked = True
            self.retries = 0
            self.rto = RTO_FIRST
            self.last_progress = time.monotonic()
            self.next_retx = self.last_progress + self.rto
            self.cond.notify_all()
        self.peer_win = window
        if ptype == T_DATA:
            if seq == (self.rcv_nxt & MASK) and not self.rcv_fin:
                if payload:
                    self.rcv_nxt += len(payload)
                    self.rcv_queued += len(payload)
                    self.inbox.append(bytes(payload))
                if flags & F_FIN:
                    self.rcv_nxt += 1
                    self.rcv_fin = True
                self.cond.notify_all()
            if not self._pump():
                self._send(T_ACKONLY, 0, self.rcv_nxt, self._rcv_window())
        else:
            self._pump()

    def keepalive(self):
        self._send(T_ACKONLY, 0, self.rcv_nxt, self._rcv_window())

    def tick_data(self, now):
        pending = self.snd_nxt > self.snd_una or len(self.buf) > 0 or (self.fin_queued and not self.fin_sent)
        if not pending:
            self.last_progress = now
            return
        if self.snd_nxt == self.snd_una and self.peer_win == 0:
            # zero window: probe with an empty DATA (not counted for give-up)
            if now >= self.next_retx:
                self.rto = min(self.rto * 2, RTO_MAX)
                self.next_retx = now + self.rto
                self._send_data(self.snd_nxt, b'')
            self.last_progress = now
            return
        if self.snd_nxt == self.snd_una:
            self._pump()
            return
        if now - self.last_progress > STREAM_GIVEUP:
            self._fail(L3PError('TIMEOUT', message='peer stopped acknowledging'), (R_TIMEOUT, ''))
            return
        if now >= self.next_retx:
            self.rto = min(self.rto * 2, RTO_MAX)
            self.next_retx = now + self.rto
            self.snd_nxt = self.snd_una        # go-back-N
            self.inflight = []
            if self.fin_sent and not self.fin_acked:
                self.fin_sent = False
            self._pump()


# ---- the endpoint ----

class Endpoint:
    """One CNET socket speaking L3P. ``listen(kind, server)`` accepts
    associations; ``open_txn`` / ``open_stream`` start them; ``ucall`` asks
    a connectionless question.

    The server object may define on_open(assoc), on_request(assoc, payload)
    -> bytes (transactions), on_close(assoc, error) and on_ureq(src, payload)
    -> bytes or None. They run on the endpoint's worker thread.

    ``allow(peer)`` limits who may open associations or get URSP;
    ``allow_keys`` (fingerprints) limits encrypted peers and makes
    encryption mandatory; ``psk`` (32 bytes) selects XXpsk3."""

    def __init__(self, node, sock=None, *, key=None, psk=None, require_encryption=False, allow=None,
                 allow_keys=None, max_assocs=8, idle=IDLE_DEFAULT, verbose=False):
        self.node = node
        self.socket = sock if sock is not None else node.alloc_socket()
        self.static = key
        self.psk = psk
        self.allow = allow
        self.allow_keys = set(k.lower() for k in allow_keys) if allow_keys else None
        self.require_encryption = require_encryption or bool(self.allow_keys)
        self.max_assocs = max_assocs
        self.idle = idle
        self.verbose = verbose
        self.lock = threading.RLock()
        self.assocs = {}              # (node, socket, conn) -> Assoc
        self.opening = {}             # (node, socket, nonce) -> Assoc (initiator)
        self.accepted = {}            # (node, socket, nonce) -> Assoc (responder)
        self.servers = {}             # kind -> server
        self.ucalls = {}              # seq -> [responses]
        self.events = queue.Queue()
        self.stopping = threading.Event()
        node.handlers[self.socket] = self.on_datagram
        self.timer = threading.Thread(target=self._run_timer, name=f'l3p-timer-{self.socket}', daemon=True)
        self.worker = threading.Thread(target=self._run_worker, name=f'l3p-worker-{self.socket}', daemon=True)
        self.timer.start()
        self.worker.start()

    def close(self):
        with self.lock:
            assocs = list(self.assocs.values()) + list(self.opening.values())
        for assoc in assocs:
            assoc.abort('endpoint closed')
        self.stopping.set()
        self.events.put(None)
        if self.node.handlers.get(self.socket) == self.on_datagram:
            del self.node.handlers[self.socket]

    def listen(self, kind, server):
        self.servers[kind] = server

    def count(self):
        with self.lock:
            return sum(1 for a in self.assocs.values() if not a.initiator)

    # -- opening --

    def _open(self, cls, net, node, sock, encrypt, verify, timeout, **extra):
        if encrypt and not HAVE_CRYPTO:
            raise L3PError('RESET', R_ENC_UNSUPPORTED, 'install the cryptography package')
        if encrypt and self.static is None:
            self.static = load_or_create_key()
        assoc = cls(self, (net, node, sock), True, **extra)
        assoc.nonce = random.randrange(1, 1 << 32)
        assoc.verify = verify
        options = 0
        hs1 = b''
        if encrypt:
            options |= OPT_ENC | (OPT_PSK if self.psk else 0)
            prologue = b'L3P/1' + struct.pack('>BIHHB', assoc.kind, assoc.nonce, net, node, sock)
            assoc.hs = Handshake(True, self.static, prologue, self.psk)
            hs1 = assoc.hs.write()
        payload = struct.pack('>II', assoc.nonce, options) + hs1
        window = assoc.open_window()
        with self.lock:
            self.opening[(node, sock, assoc.nonce)] = assoc
            assoc.ctrl = _Retx(lambda: assoc._send(T_OPEN, 0, 0, window, payload, conn=0), OPEN_GIVEUP)
            assoc._send(T_OPEN, 0, 0, window, payload, conn=0)
        try:
            assoc.wait_open(timeout)
        except TimeoutError:
            assoc.abort('open timed out')
            raise L3PError('TIMEOUT', message='open timed out')
        return assoc

    def open_txn(self, net, node, sock, *, window=TXN_WINDOW_MAX, encrypt=False, verify=None,
                 timeout=OPEN_GIVEUP + 1):
        return self._open(TxnAssoc, net, node, sock, encrypt, verify, timeout, window=window)

    def open_stream(self, net, node, sock, *, encrypt=False, verify=None, timeout=OPEN_GIVEUP + 1):
        return self._open(StreamAssoc, net, node, sock, encrypt, verify, timeout)

    # -- connectionless --

    def ucall(self, net, node, sock, payload, *, wait=0.5, timeout=3.0):
        """Sends UREQ and returns [(src, payload)]. Unicast returns at the
        first answer (or [] after ``timeout``); broadcast collects for ``wait``."""
        payload = bytes(payload)
        if len(payload) > MAX_PLAIN:
            raise ValueError('UREQ too large')
        seq = random.randrange(1, 1 << 32)
        broadcast = node == BROADCAST
        answers = []
        header = HEADER.pack(L3P_VERSION << 4 | KIND_TXN, T_UREQ, 0, 0, 0, seq, 0, 0, 0)
        with self.lock:
            self.ucalls[seq] = answers
        try:
            start = time.monotonic()
            end = start + (wait if broadcast else timeout)
            interval = RTO_FIRST
            next_send = start
            while True:
                now = time.monotonic()
                if now >= end or (answers and not broadcast):
                    break
                if now >= next_send:
                    self.node.send_datagram(net, node, sock, self.socket, PROTO_L3P, header + payload)
                    next_send = now + interval
                    interval = min(interval * 2, RTO_MAX)
                time.sleep(0.01)
        finally:
            with self.lock:
                del self.ucalls[seq]
        return list(answers)

    # -- receiving --

    def on_datagram(self, snet, snode, ssock, proto, body, dnet, dnode):
        if proto != PROTO_L3P or len(body) < HEADER.size:
            return
        header = body[:HEADER.size]
        ver_kind, ptype, flags, rsv, conn, seq, ack, window, rsv2 = HEADER.unpack(header)
        kind = ver_kind & 0x0F
        if ver_kind >> 4 != L3P_VERSION or rsv or rsv2 or kind not in VALID_TYPES or ptype not in VALID_TYPES[kind]:
            return
        if flags & ~(F_ENC | F_FIN) or (flags & F_FIN and ptype != T_DATA):
            return
        payload = body[HEADER.size:]
        peer = (snet, snode, ssock)
        with self.lock:
            if ptype == T_UREQ:
                self._on_ureq(peer, seq, payload, dnode == BROADCAST, flags)
                return
            if ptype == T_URSP:
                answers = self.ucalls.get(seq)
                if answers is not None and not any(src == peer for src, _ in answers):
                    answers.append((peer, payload))
                return
            if ptype == T_OPEN:
                if conn == 0 and dnode != BROADCAST:
                    self._on_open(peer, kind, window, payload, dnet, dnode)
                return
            if ptype == T_ACCEPT:
                if len(payload) >= 4:
                    nonce = struct.unpack('>I', payload[:4])[0]
                    assoc = self.opening.get((snode, ssock, nonce))
                    if assoc and _same_peer(assoc.peer, peer) and assoc.kind == kind and conn:
                        del self.opening[(snode, ssock, nonce)]
                        assoc.conn = conn
                        self.assocs[(snode, ssock, conn)] = assoc
                        assoc.last_rx = time.monotonic()
                        assoc.on_accept(payload, window)
                return
            if ptype == T_RESET and conn == 0:
                assoc = self.opening.get((snode, ssock, seq))
                if assoc and _same_peer(assoc.peer, peer):
                    reason, message = parse_reset(payload)
                    assoc._fail(L3PError('RESET', reason, message))
                return
            assoc = self.assocs.get((snode, ssock, conn))
            if assoc is None or not _same_peer(assoc.peer, peer) or assoc.kind != kind:
                if ptype in (T_REQ, T_DATA, T_HS, T_CLOSE) and conn:
                    self._reset(peer, kind, conn, R_UNKNOWN_CONN)
                return
            assoc.on_packet(ptype, flags, seq, ack, window, payload, header)

    def _reset(self, peer, kind, conn, reason, message='', seq=0):
        header = HEADER.pack(L3P_VERSION << 4 | kind, T_RESET, 0, 0, conn, seq, 0, 0, 0)
        net, node, sock = peer
        self.node.send_datagram(net, node, sock, self.socket, PROTO_L3P, header + reset_payload(reason, message))

    def _allowed(self, peer):
        return self.allow is None or self.allow(peer)

    def _on_open(self, peer, kind, window, payload, dnet, dnode):
        if len(payload) < 8:
            return
        nonce, options = struct.unpack('>II', payload[:8])
        net, node, sock = peer
        old = self.accepted.get((node, sock, nonce))
        if old is not None and old.state not in ('closed', 'broken'):
            old._send(T_ACCEPT, 0, 0, old.open_window(), old.accept_packet)
            return

        def refuse(reason, message=''):
            if self.verbose:
                log(f'{format_addr(net, node)}:{sock}', 'refused:', REASONS[reason], message)
            self._reset(peer, kind, 0, reason, message, seq=nonce)

        if nonce == 0 or options & ~(OPT_ENC | OPT_PSK) or (options & OPT_PSK and not options & OPT_ENC):
            return
        server = self.servers.get(kind)
        if server is None:
            return refuse(R_NO_LISTENER)
        if not self._allowed(peer):
            return refuse(R_REFUSED, 'not allowed')
        if options & OPT_ENC and (not HAVE_CRYPTO or self.static is None):
            return refuse(R_ENC_UNSUPPORTED)
        if self.require_encryption and not options & OPT_ENC:
            return refuse(R_ENC_REQUIRED, 'encryption required')
        if bool(options & OPT_PSK) != bool(self.psk) and options & OPT_ENC:
            return refuse(R_AUTH_FAILED, 'pre-shared key mismatch')
        if self.count() >= self.max_assocs:
            return refuse(R_BUSY, 'too many associations')
        cls = TxnAssoc if kind == KIND_TXN else StreamAssoc
        assoc = cls(self, peer, False)
        if kind == KIND_TXN:
            assoc.window = max(1, min(window or TXN_WINDOW_MAX, TXN_WINDOW_MAX))
        else:
            assoc.peer_win = window
        assoc.nonce = nonce
        assoc.options = options
        assoc.idle = self.idle
        while True:
            conn = random.randrange(1, 1 << 32)
            if (node, sock, conn) not in self.assocs:
                break
        assoc.conn = conn
        hs2 = b''
        if options & OPT_ENC:
            prologue = b'L3P/1' + struct.pack('>BIHHB', kind, nonce, dnet, dnode, self.socket)
            assoc.hs = Handshake(False, self.static, prologue, self.psk)
            try:
                assoc.hs.read(payload[8:])
                hs2 = assoc.hs.write()
            except (ValueError, InvalidTag):
                return refuse(R_AUTH_FAILED, 'bad handshake')
            assoc.state = 'handshake'
        else:
            assoc.state = 'open'
        assoc.accept_packet = struct.pack('>IIHH', nonce, options, self.idle, 0) + hs2
        self.assocs[(node, sock, conn)] = assoc
        for key in [k for k, a in self.accepted.items() if a.state in ('closed', 'broken')]:
            del self.accepted[key]
        self.accepted[(node, sock, nonce)] = assoc
        assoc._send(T_ACCEPT, 0, 0, assoc.open_window(), assoc.accept_packet)
        if self.verbose:
            log(assoc.name, 'open', 'encrypted' if options & OPT_ENC else 'plain')
        if assoc.state == 'open':
            self._event(('open', assoc))

    def _on_ureq(self, peer, seq, payload, broadcast, flags):
        if flags & F_ENC:
            return
        server = self.servers.get(KIND_TXN)
        if server is None or not hasattr(server, 'on_ureq') or not self._allowed(peer):
            return
        self._event(('ureq', peer, seq, payload, random.uniform(0, 0.05) if broadcast else 0))

    def _forget(self, assoc):
        net, node, sock = assoc.peer
        if self.assocs.get((node, sock, assoc.conn)) is assoc:
            del self.assocs[(node, sock, assoc.conn)]
        if self.opening.get((node, sock, assoc.nonce)) is assoc:
            del self.opening[(node, sock, assoc.nonce)]

    def _event(self, event):
        self.events.put(event)

    # -- threads --

    def _run_timer(self):
        while not self.stopping.wait(TICK):
            now = time.monotonic()
            with self.lock:
                assocs = list(self.assocs.values()) + list(self.opening.values())
                for assoc in assocs:
                    try:
                        assoc.tick(now)
                    except Exception as error:
                        log('l3p tick error:', repr(error))

    def _run_worker(self):
        while True:
            event = self.events.get()
            if event is None:
                return
            try:
                self._dispatch(event)
            except Exception as error:
                log('l3p worker error:', repr(error))

    def _dispatch(self, event):
        what = event[0]
        if what == 'ureq':
            _, peer, seq, payload, delay = event
            server = self.servers.get(KIND_TXN)
            answer = server.on_ureq(peer, payload) if server else None
            if answer is None:
                return
            if delay:
                time.sleep(delay)
            header = HEADER.pack(L3P_VERSION << 4 | KIND_TXN, T_URSP, 0, 0, 0, seq, 0, 0, 0)
            net, node, sock = peer
            self.node.send_datagram(net, node, sock, self.socket, PROTO_L3P, header + bytes(answer)[:MAX_PLAIN])
            return
        assoc = event[1]
        server = self.servers.get(assoc.kind)
        if server is None:
            return
        if what == 'open':
            if hasattr(server, 'on_open'):
                server.on_open(assoc)
        elif what == 'req':
            _, _, xid, payload = event
            try:
                answer = server.on_request(assoc, payload)
            except Exception as error:
                log(assoc.name, 'request error:', repr(error))
                assoc.abort('server error')
                return
            assoc.reply(xid, answer)
        elif what == 'close':
            if hasattr(server, 'on_close'):
                server.on_close(assoc, event[2])
