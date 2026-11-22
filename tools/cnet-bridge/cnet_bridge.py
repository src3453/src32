#!/usr/bin/env python3
"""CNET TCP bridge: telnet <-> telnet and telnet <-> SSH (doc/spec_NET_bridge.md).

The bridge is one node on a CNET UDP segment (doc/spec_NET.md §8.3). A CPT32
client (Foundation's telnet / ssh) opens a reliable stream to it with PROTO
0x40 and names a target:

    telnet://HOST[:PORT]       bytes are relayed to a TCP telnet server
    ssh://[USER@]HOST[:PORT]   the bridge acts as a telnet server and an SSH
                               client (needs paramiko)

Example, with an emulator whose config has
``udp: {bind: "127.0.0.1:32032", peers: ["127.0.0.1:32040"], segment_id: 0}``:

    python tools/cnet-bridge/cnet_bridge.py --bind 127.0.0.1:32040 \\
        --peer 127.0.0.1:32032 --net 1 --allow localhost --allow '*.example.org:22'

Nothing is reachable until --allow (or --allow-any) says so.
"""
import argparse
import base64
import fnmatch
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

# ---- stream protocol (doc/spec_NET_bridge.md §3) ----

PROTO_STREAM = 0x40
BRIDGE_SOCKET = 23
BRIDGE_NODE = 0xFF01
T_OPEN, T_ACCEPT, T_DATA, T_ACK, T_RESET = 1, 2, 3, 4, 5
FLAG_FIN = 0x01
HEADER = struct.Struct('>BBHIIHH')
MAX_DATA = MTU - HEADER.size          # 1008
WINDOW = 16 * 1024                    # what the bridge advertises
MAX_INFLIGHT = 4 * MAX_DATA           # the card queues 8 datagrams per socket
SEND_BUFFER = 64 * 1024               # backends block above this
RTO = 0.3
MAX_RETRIES = 30
KEEPALIVE = 15.0
DEAD = 60.0
MASK = 0xFFFFFFFF


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


def stream_packet(kind, conn, seq=0, ack=0, window=0, data=b'', flags=0):
    return HEADER.pack(kind, flags, conn, seq & MASK, ack & MASK, window, 0) + data


def parse_hostport(text, default_port=None):
    host, sep, port = text.rpartition(':')
    if not sep:
        if default_port is None:
            raise ValueError(f'{text}: expected HOST:PORT')
        return text, default_port
    return host.strip('[]') or '0.0.0.0', int(port)


def log(*args):
    print(time.strftime('%H:%M:%S'), *args, file=sys.stderr, flush=True)


# ---- physical layer: one UDP segment ----

class Segment:
    """Sends link frames to the peers (and the multicast group) and hands
    received ones to on_frame. Unicast senders of valid packets are learned
    as peers when no multicast group is used, so an emulator only has to
    list the bridge in its udp.peers."""

    def __init__(self, bind, segment_id=0, multicast=None, peers=(), learn=True):
        self.segment_id = segment_id
        self.sender = random.getrandbits(32)
        self.peers = list(peers)
        self.multicast = multicast
        self.learn = learn and not multicast
        self.learned = {}
        self.on_frame = None
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
        self.thread.join(timeout=2)
        self.sock.close()

    def send(self, frame):
        packet = UDP_MAGIC + struct.pack('>BBHI', 1, 0, self.segment_id, self.sender) + frame
        targets = list(self.peers)
        if self.multicast:
            targets.append(self.multicast)
        now = time.monotonic()
        targets += [a for a, seen in list(self.learned.items()) if now - seen < 120 and a not in targets]
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
                except Exception as error:  # a bad frame must not stop the bridge
                    log('frame error:', repr(error))


# ---- the node: link layer, echo, dispatch ----

class Node:
    def __init__(self, segment, net=0, node=BRIDGE_NODE):
        self.segment = segment
        self.net = net
        self.node = node
        self.handlers = {}  # socket -> fn(src_net, src_node, src_socket, proto, body)
        segment.on_frame = self.on_frame

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
            handler(snet, snode, ssock, proto, body)


# ---- one reliable stream (doc/spec_NET_bridge.md §3.2) ----

class Aborted(Exception):
    pass


class Stream:
    """The bridge's end of one connection. Backends call write(),
    close_write() and read(); the node thread calls on_packet() and the
    timer thread tick()."""

    def __init__(self, send, conn):
        self.send_raw = send          # fn(payload bytes)
        self.conn = conn
        self.cond = threading.Condition()
        self.buf = bytearray()        # unacknowledged and unsent bytes, from snd_una
        self.snd_una = 0
        self.snd_nxt = 0
        self.fin_queued = False
        self.fin_sent = False
        self.fin_acked = False
        self.rcv_nxt = 0
        self.rcv_fin = False
        self.peer_window = MAX_INFLIGHT
        self.inbox = queue.Queue()    # bytes, None at EOF
        self.aborted = None           # reason once reset
        now = time.monotonic()
        self.last_rx = now
        self.last_progress = now
        self.last_keepalive = now
        self.retries = 0

    # -- used by backends --

    def write(self, data):
        data = bytes(data)
        with self.cond:
            while data:
                while len(self.buf) >= SEND_BUFFER and not self.aborted:
                    self.cond.wait(0.5)
                if self.aborted:
                    raise Aborted(self.aborted)
                if self.fin_queued:
                    return
                room = SEND_BUFFER - len(self.buf)
                self.buf += data[:room]
                data = data[room:]
                self._pump()

    def close_write(self):
        with self.cond:
            if not self.fin_queued and not self.aborted:
                self.fin_queued = True
                self._pump()

    def read(self, timeout=None):
        """Next received bytes, b'' at EOF. Raises Aborted or queue.Empty."""
        item = self.inbox.get(timeout=timeout)
        if isinstance(item, Aborted):
            self.inbox.put(item)
            raise item
        if item is None:
            self.inbox.put(None)
            return b''
        return item

    def abort(self, reason, notify=True):
        with self.cond:
            if self.aborted:
                return
            self.aborted = reason
            self.cond.notify_all()
        if notify:
            self.send_raw(stream_packet(T_RESET, self.conn, data=reason.encode('utf-8')[:MAX_DATA]))
        self.inbox.put(Aborted(reason))

    @property
    def finished(self):
        return self.aborted is not None or (self.fin_acked and self.rcv_fin)

    # -- protocol --

    def _send_segment(self, seq, data, flags=0):
        self.send_raw(stream_packet(T_DATA, self.conn, seq, self.rcv_nxt, WINDOW, data, flags))

    def _send_ack(self):
        self.send_raw(stream_packet(T_ACK, self.conn, 0, self.rcv_nxt, WINDOW))

    def _pump(self):
        if self.aborted:
            return
        limit = self.snd_una + min(self.peer_window, MAX_INFLIGHT)
        end = self.snd_una + len(self.buf)
        while self.snd_nxt < end and self.snd_nxt < limit:
            start = self.snd_nxt - self.snd_una
            n = min(MAX_DATA, end - self.snd_nxt, limit - self.snd_nxt)
            self._send_segment(self.snd_nxt, bytes(self.buf[start:start + n]))
            self.snd_nxt += n
        if self.fin_queued and not self.fin_sent and self.snd_nxt == end:
            self._send_segment(end, b'', FLAG_FIN)
            self.fin_sent = True
            self.snd_nxt = end + 1

    def on_packet(self, kind, flags, seq, ack, window, data):
        with self.cond:
            if self.aborted:
                return
            self.last_rx = time.monotonic()
            # Acknowledgement (every DATA and ACK carries one).
            in_flight = self.snd_nxt - self.snd_una
            delta = (ack - self.snd_una) & MASK
            if 0 < delta <= in_flight:
                taken = min(delta, len(self.buf))
                del self.buf[:taken]
                self.snd_una += delta
                if self.fin_sent and self.snd_una == self.snd_nxt:
                    self.fin_acked = True
                self.retries = 0
                self.last_progress = self.last_rx
                self.cond.notify_all()
            self.peer_window = window
            if kind == T_DATA:
                if seq == (self.rcv_nxt & MASK) and not self.rcv_fin:
                    if data:
                        self.rcv_nxt += len(data)
                        self.inbox.put(bytes(data))
                    if flags & FLAG_FIN:
                        self.rcv_nxt += 1
                        self.rcv_fin = True
                        self.inbox.put(None)
                self._send_ack()
            self._pump()

    def tick(self, now):
        with self.cond:
            if self.aborted:
                return
            if self.snd_nxt > self.snd_una:
                if now - self.last_progress >= RTO * min(1 + self.retries, 4):
                    self.retries += 1
                    if self.retries > MAX_RETRIES:
                        reason = 'peer stopped answering'
                    else:
                        self.last_progress = now
                        self.snd_nxt = self.snd_una  # go-back-N
                        if self.fin_sent and not self.fin_acked:
                            self.fin_sent = False
                        self._pump()
                        return
                else:
                    return
            elif now - self.last_rx >= DEAD:
                reason = 'peer stopped answering'
            else:
                if now - self.last_rx >= KEEPALIVE and now - self.last_keepalive >= KEEPALIVE:
                    self.last_keepalive = now
                    self._send_segment(self.snd_nxt, b'')
                return
        self.abort(reason)


# ---- telnet server side for SSH sessions (RFC 854, 1073, 1091) ----

IAC, DONT, DO, WONT, WILL, SB, SE = 255, 254, 253, 252, 251, 250, 240
OPT_BINARY, OPT_ECHO, OPT_SGA, OPT_TTYPE, OPT_NAWS = 0, 1, 3, 24, 31
TTYPE_IS, TTYPE_SEND = 0, 1


class TelnetServer:
    """Strips telnet commands from the client's bytes and tracks NAWS and
    TTYPE. feed() returns the user data with CR LF / CR NUL folded to CR."""

    def __init__(self, write):
        self.write = write  # fn(raw bytes) towards the client
        self.state = 0      # 0 data, 1 IAC, 2 option verb, 3 SB, 4 SB IAC
        self.verb = 0
        self.sb = bytearray()
        self.after_cr = False
        self.cols, self.rows = 80, 24
        self.term = None
        self.on_resize = None
        self.ttype_event = threading.Event()

    def start(self):
        self.write(bytes([IAC, WILL, OPT_ECHO, IAC, WILL, OPT_SGA, IAC, DO, OPT_NAWS, IAC, DO, OPT_TTYPE]))

    @staticmethod
    def escape(data):
        return bytes(data).replace(b'\xff', b'\xff\xff')

    def _option(self, verb, opt):
        if verb == WILL:
            if opt == OPT_TTYPE:
                self.write(bytes([IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]))
            elif opt not in (OPT_NAWS, OPT_BINARY):
                self.write(bytes([IAC, DONT, opt]))
        elif verb == WONT:
            if opt == OPT_TTYPE:
                self.ttype_event.set()
        elif verb == DO:
            if opt not in (OPT_ECHO, OPT_SGA, OPT_BINARY):
                self.write(bytes([IAC, WONT, opt]))

    def _subneg(self, sb):
        if len(sb) >= 5 and sb[0] == OPT_NAWS:
            self.cols, self.rows = struct.unpack('>HH', bytes(sb[1:5]))
            if self.on_resize:
                self.on_resize(self.cols, self.rows)
        elif len(sb) >= 2 and sb[0] == OPT_TTYPE and sb[1] == TTYPE_IS:
            name = bytes(sb[2:]).decode('ascii', 'replace').strip().lower()
            if name and self.term is None:
                self.term = name
            self.ttype_event.set()

    def feed(self, data):
        out = bytearray()
        for byte in data:
            if self.state == 0:
                if byte == IAC:
                    self.state = 1
                    continue
                if self.after_cr and byte in (0, 10):
                    self.after_cr = False
                    continue
                self.after_cr = byte == 13
                out.append(byte)
            elif self.state == 1:
                if byte == IAC:
                    out.append(IAC)
                    self.after_cr = False
                    self.state = 0
                elif byte in (WILL, WONT, DO, DONT):
                    self.verb = byte
                    self.state = 2
                elif byte == SB:
                    self.sb.clear()
                    self.state = 3
                else:
                    self.state = 0
            elif self.state == 2:
                self._option(self.verb, byte)
                self.state = 0
            elif self.state == 3:
                if byte == IAC:
                    self.state = 4
                elif len(self.sb) < 256:
                    self.sb.append(byte)
            else:  # SB IAC
                if byte == SE:
                    self._subneg(self.sb)
                    self.state = 0
                else:
                    if byte == IAC and len(self.sb) < 256:
                        self.sb.append(IAC)
                    self.state = 3
        return bytes(out)


class Dialog:
    """Line input and output over the telnet stream, before the SSH shell
    starts. The bridge has said WILL ECHO, so it echoes typed text itself."""

    def __init__(self, stream, telnet):
        self.stream = stream
        self.telnet = telnet
        self.pending = bytearray()

    def say(self, text):
        self.stream.write(TelnetServer.escape(text.replace('\r\n', '\n').replace('\n', '\r\n').encode('utf-8')))

    def read_line(self, prompt, echo=True, timeout=120.0):
        self.say(prompt)
        line = bytearray()
        deadline = time.monotonic() + timeout
        while True:
            while not self.pending:
                left = deadline - time.monotonic()
                if left <= 0:
                    raise Aborted('login timed out')
                try:
                    data = self.stream.read(timeout=left)
                except queue.Empty:
                    continue
                if not data:
                    raise Aborted('client closed the connection')
                self.pending += self.telnet.feed(data)
            byte = self.pending.pop(0)
            if byte in (13, 10):
                self.say('\n')
                return line.decode('utf-8', 'replace')
            if byte in (3, 4):
                self.say('^C\n' if byte == 3 else '\n')
                raise Aborted('cancelled')
            if byte in (8, 127):
                if line:
                    while line and (line.pop() & 0xC0) == 0x80:
                        pass
                    if echo:
                        self.say('\b \b')
                continue
            if byte < 0x20:
                continue
            line.append(byte)
            if echo:
                self.stream.write(TelnetServer.escape(bytes([byte])))

    def take_pending(self):
        data = bytes(self.pending)
        self.pending.clear()
        return data


# ---- backends ----

def default_known_hosts():
    return Path.home() / '.cpt32' / 'cnet_bridge_known_hosts'


class Session:
    def __init__(self, bridge, key, conn, target):
        self.bridge = bridge
        self.key = key
        self.conn = conn
        self.target = target
        net, node, sock = key
        self.stream = Stream(lambda payload: bridge.node.send_datagram(
            net, node, sock, BRIDGE_SOCKET, PROTO_STREAM, payload), conn)
        self.accepted = False
        self.name = f'{net:04X}.{node:04X}/{sock} #{conn:04X}'

    def accept(self):
        self.accepted = True
        self.send_accept()

    def send_accept(self):
        net, node, sock = self.key
        self.bridge.node.send_datagram(net, node, sock, BRIDGE_SOCKET, PROTO_STREAM,
                                       stream_packet(T_ACCEPT, self.conn, 0, 0, WINDOW))

    def start(self):
        threading.Thread(target=self.run_safely, name=f'session {self.name}', daemon=True).start()

    def run_safely(self):
        try:
            self.run()
        except Aborted as error:
            log(self.name, 'aborted:', error)
        except Exception as error:
            log(self.name, 'error:', repr(error))
            self.stream.abort(f'bridge error: {error}')

    def run(self):
        raise NotImplementedError


class TelnetSession(Session):
    """telnet://: the client's bytes go to TCP unchanged (telnet <-> telnet)."""

    def run(self):
        _, host, port, _ = self.target
        try:
            tcp = socket.create_connection((host, port), timeout=10)
        except OSError as error:
            self.stream.abort(f'{host}:{port}: {error.strerror or error}')
            return
        tcp.settimeout(None)
        log(self.name, f'connected to telnet://{host}:{port}')
        self.accept()
        threading.Thread(target=self.upstream, args=(tcp,), daemon=True).start()
        try:
            while True:
                data = tcp.recv(4096)
                if not data:
                    break
                self.stream.write(data)
            self.stream.close_write()
        except Aborted:
            pass
        except OSError:
            self.stream.close_write()
        finally:
            tcp.close()
        log(self.name, 'closed')

    def upstream(self, tcp):
        try:
            while True:
                data = self.stream.read()
                if not data:
                    tcp.shutdown(socket.SHUT_WR)
                    return
                tcp.sendall(data)
        except (Aborted, OSError):
            try:
                tcp.close()
            except OSError:
                pass


class SshSession(Session):
    """ssh://: the bridge is the client's telnet server and an SSH client."""

    def run(self):
        try:
            import paramiko
        except ImportError:
            self.stream.abort('this bridge has no SSH support (pip install paramiko)')
            return
        _, host, port, user = self.target
        self.accept()
        telnet = TelnetServer(lambda raw: self.stream.write(raw))
        telnet.start()
        dialog = Dialog(self.stream, telnet)
        try:
            tcp = socket.create_connection((host, port), timeout=10)
        except OSError as error:
            dialog.say(f'ssh: connect to {host} port {port}: {error.strerror or error}\n')
            self.stream.close_write()
            return
        transport = paramiko.Transport(tcp)
        try:
            transport.start_client(timeout=15)
            if not self.check_host_key(paramiko, transport, host, port, dialog):
                self.stream.close_write()
                return
            if not user:
                user = dialog.read_line('login as: ').strip()
                if not user:
                    self.stream.close_write()
                    return
            if not self.authenticate(paramiko, transport, user, host, dialog):
                dialog.say('ssh: authentication failed\n')
                self.stream.close_write()
                return
            log(self.name, f'ssh {user}@{host}:{port} authenticated')
            telnet.ttype_event.wait(1.0)
            channel = transport.open_session()
            channel.get_pty(term=telnet.term or 'ansi', width=telnet.cols, height=telnet.rows)
            channel.invoke_shell()
            telnet.on_resize = lambda cols, rows: channel.resize_pty(width=cols, height=rows)
            first = dialog.take_pending()
            if first:
                channel.sendall(first)
            threading.Thread(target=self.upstream, args=(channel, telnet), daemon=True).start()
            while True:
                data = channel.recv(4096)
                if not data:
                    break
                self.stream.write(TelnetServer.escape(data))
            self.stream.close_write()
        except paramiko.SSHException as error:
            try:
                dialog.say(f'ssh: {error}\n')
                self.stream.close_write()
            except Aborted:
                pass
        finally:
            transport.close()
        log(self.name, 'closed')

    def upstream(self, channel, telnet):
        try:
            while True:
                data = self.stream.read()
                if not data:
                    channel.shutdown_write()
                    return
                data = telnet.feed(data)
                if data:
                    channel.sendall(data)
        except (Aborted, OSError):
            channel.close()
        except Exception:
            channel.close()

    def check_host_key(self, paramiko, transport, host, port, dialog):
        key = transport.get_remote_server_key()
        name = host if port == 22 else f'[{host}]:{port}'
        known = paramiko.HostKeys()
        for path in (Path.home() / '.ssh' / 'known_hosts', self.bridge.known_hosts):
            if path.is_file():
                try:
                    known.load(str(path))
                except (OSError, paramiko.SSHException):
                    pass
        entry = known.lookup(name)
        if entry and key.get_name() in entry:
            if entry[key.get_name()] == key:
                return True
            dialog.say(f'WARNING: the {key.get_name()} host key of {name} has changed!\n'
                       'Remove the old key from the known_hosts file if the change is expected.\n')
            return False
        fingerprint = base64.b64encode(__import__('hashlib').sha256(key.asbytes()).digest()).decode().rstrip('=')
        dialog.say(f"The authenticity of host '{name}' can't be established.\n"
                   f'{key.get_name()} key fingerprint is SHA256:{fingerprint}.\n')
        answer = dialog.read_line('Are you sure you want to continue connecting (yes/no)? ').strip().lower()
        if answer != 'yes':
            return False
        path = self.bridge.known_hosts
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            with open(path, 'a', encoding='ascii') as f:
                f.write(f'{name} {key.get_name()} {key.get_base64()}\n')
            dialog.say(f"Warning: permanently added '{name}' to the bridge's known hosts.\n")
        except OSError as error:
            dialog.say(f'Warning: could not save the host key: {error}\n')
        return True

    def authenticate(self, paramiko, transport, user, host, dialog):
        try:
            transport.auth_none(user)
            return True
        except paramiko.BadAuthenticationType as error:
            allowed = set(error.allowed_types)
        except paramiko.SSHException:
            allowed = {'publickey', 'password', 'keyboard-interactive'}
        if 'publickey' in allowed:
            for key in self.keys(paramiko):
                try:
                    transport.auth_publickey(user, key)
                    if transport.is_authenticated():
                        return True
                except paramiko.SSHException:
                    continue
        if 'password' in allowed:
            for _ in range(3):
                password = dialog.read_line(f"{user}@{host}'s password: ", echo=False)
                try:
                    transport.auth_password(user, password)
                    if transport.is_authenticated():
                        return True
                except paramiko.BadAuthenticationType:
                    break
                except paramiko.AuthenticationException:
                    dialog.say('Permission denied, please try again.\n')
        if 'keyboard-interactive' in allowed:
            def handler(title, instructions, prompts):
                if title:
                    dialog.say(title + '\n')
                if instructions:
                    dialog.say(instructions + '\n')
                return [dialog.read_line(prompt, echo=echo) for prompt, echo in prompts]
            try:
                transport.auth_interactive(user, handler)
                return transport.is_authenticated()
            except paramiko.SSHException:
                return False
        return False

    def keys(self, paramiko):
        try:
            yield from paramiko.Agent().get_keys()
        except Exception:
            pass
        paths = list(self.bridge.identities)
        ssh = Path.home() / '.ssh'
        paths += [ssh / name for name in ('id_ed25519', 'id_ecdsa', 'id_rsa')]
        kinds = [getattr(paramiko, n) for n in ('Ed25519Key', 'ECDSAKey', 'RSAKey') if hasattr(paramiko, n)]
        for path in paths:
            if not Path(path).is_file():
                continue
            for kind in kinds:
                try:
                    yield kind.from_private_key_file(str(path))
                    break
                except Exception:  # encrypted, or another key type
                    continue


# ---- the bridge ----

def parse_target(text):
    """('telnet'|'ssh', host, port, user or None), or raises ValueError."""
    scheme, sep, rest = text.partition('://')
    if not sep or scheme not in ('telnet', 'ssh'):
        raise ValueError('expected telnet://HOST[:PORT] or ssh://[USER@]HOST[:PORT]')
    rest = rest.rstrip('/')
    user = None
    if '@' in rest:
        user, rest = rest.rsplit('@', 1)
        if scheme != 'ssh' or not user:
            raise ValueError('a user name is only allowed with ssh://')
    port = 23 if scheme == 'telnet' else 22
    if rest.startswith('['):
        host, _, tail = rest[1:].partition(']')
        if tail:
            if not tail.startswith(':'):
                raise ValueError('bad address')
            port = int(tail[1:])
    elif rest.count(':') == 1:
        host, port_text = rest.split(':')
        port = int(port_text)
    else:
        host = rest
    if not host or not 0 < port < 65536 or any(c.isspace() for c in host):
        raise ValueError('bad host or port')
    return scheme, host, port, user


def allowed(patterns, host, port):
    host = host.lower()
    for pattern in patterns:
        pattern = pattern.lower()
        if pattern == '*':
            return True
        if pattern.startswith('[') or pattern.count(':') == 1:
            phost, _, pport = pattern.rpartition(':')
            phost = phost.strip('[]')
            if fnmatch.fnmatchcase(host, phost) and fnmatch.fnmatchcase(str(port), pport):
                return True
        elif fnmatch.fnmatchcase(host, pattern.strip('[]')):
            return True
    return False


class Bridge:
    def __init__(self, node, allow=(), allow_any=False, max_sessions=8, known_hosts=None, identities=()):
        self.node = node
        self.allow = list(allow)
        self.allow_any = allow_any
        self.max_sessions = max_sessions
        self.known_hosts = Path(known_hosts) if known_hosts else default_known_hosts()
        self.identities = list(identities)
        self.sessions = {}  # (net, node, socket) -> Session
        self.lock = threading.Lock()
        self.stop = threading.Event()
        node.handlers[BRIDGE_SOCKET] = self.on_datagram
        self.timer = threading.Thread(target=self.run_timer, name='cnet-timer', daemon=True)

    def start(self):
        self.timer.start()

    def close(self):
        self.stop.set()
        with self.lock:
            sessions = list(self.sessions.values())
            self.sessions.clear()
        for session in sessions:
            session.stream.abort('bridge shutting down')

    def run_timer(self):
        while not self.stop.wait(0.05):
            now = time.monotonic()
            with self.lock:
                sessions = list(self.sessions.items())
            for key, session in sessions:
                session.stream.tick(now)
                if session.stream.finished:
                    with self.lock:
                        if self.sessions.get(key) is session:
                            del self.sessions[key]

    def reset(self, net, node, sock, conn, reason):
        self.node.send_datagram(net, node, sock, BRIDGE_SOCKET, PROTO_STREAM,
                                stream_packet(T_RESET, conn, data=reason.encode('utf-8')[:MAX_DATA]))

    def on_datagram(self, net, node, sock, proto, body):
        if proto != PROTO_STREAM or len(body) < HEADER.size:
            return
        kind, flags, conn, seq, ack, window, _ = HEADER.unpack(body[:HEADER.size])
        data = body[HEADER.size:]
        key = (net, node, sock)
        with self.lock:
            session = self.sessions.get(key)
        if kind == T_OPEN:
            self.open(key, conn, data, session)
            return
        if not session or session.conn != conn:
            if kind != T_RESET:
                self.reset(net, node, sock, conn, 'no such connection')
            return
        if kind == T_RESET:
            session.stream.abort('reset by client', notify=False)
            log(session.name, 'reset by client')
        elif kind in (T_DATA, T_ACK) and session.accepted:
            session.stream.on_packet(kind, flags, seq, ack, window, data)

    def open(self, key, conn, data, old):
        net, node, sock = key
        if old and old.conn == conn:
            if old.accepted:
                old.send_accept()
            return
        if old:
            old.stream.abort('replaced by a new connection', notify=False)
        if not conn:
            return
        try:
            target = parse_target(data.decode('utf-8'))
        except (UnicodeDecodeError, ValueError) as error:
            self.reset(net, node, sock, conn, f'bad target: {error}')
            return
        scheme, host, port, _ = target
        if not (self.allow_any or allowed(self.allow, host, port)):
            log(f'{net:04X}.{node:04X}/{sock}', f'refused {scheme}://{host}:{port} (not allowed)')
            self.reset(net, node, sock, conn, f'{host}:{port}: not allowed by the bridge')
            return
        with self.lock:
            if old is not None and self.sessions.get(key) is old:
                del self.sessions[key]
            if len(self.sessions) >= self.max_sessions:
                busy = True
            else:
                busy = False
                session = (SshSession if scheme == 'ssh' else TelnetSession)(self, key, conn, target)
                self.sessions[key] = session
        if busy:
            self.reset(net, node, sock, conn, 'bridge busy')
            return
        log(session.name, f'open {scheme}://{host}:{port}')
        session.start()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bind', default='0.0.0.0:32032', help='UDP address of the segment (HOST:PORT)')
    parser.add_argument('--multicast', help='multicast group of the segment (GROUP:PORT)')
    parser.add_argument('--peer', action='append', default=[], help='unicast peer (HOST:PORT), repeatable')
    parser.add_argument('--segment-id', type=lambda t: int(t, 0), default=0)
    parser.add_argument('--net', type=lambda t: int(t, 0), default=0, help='network number of the segment')
    parser.add_argument('--node', type=lambda t: int(t, 16), default=BRIDGE_NODE,
                        help='bridge node number in hex (default FF01)')
    parser.add_argument('--allow', action='append', default=[],
                        help='permitted target: HOST, HOST:PORT, wildcards allowed; repeatable')
    parser.add_argument('--allow-any', action='store_true', help='permit every target')
    parser.add_argument('--max-sessions', type=int, default=8)
    parser.add_argument('--known-hosts', type=Path, default=default_known_hosts(),
                        help='SSH host keys accepted through the bridge')
    parser.add_argument('--identity', action='append', default=[], type=Path, help='SSH private key file')
    args = parser.parse_args(argv)
    if not 0 <= args.net <= 0xFFFE or not 1 <= args.node <= 0xFFFE:
        parser.error('--net or --node out of range')
    if not args.allow and not args.allow_any:
        log('warning: no --allow given; every connection will be refused')
    segment = Segment(parse_hostport(args.bind), args.segment_id,
                      parse_hostport(args.multicast) if args.multicast else None,
                      [parse_hostport(p) for p in args.peer])
    node = Node(segment, args.net, args.node)
    bridge = Bridge(node, args.allow, args.allow_any, args.max_sessions, args.known_hosts, args.identity)
    segment.start()
    bridge.start()
    log(f'CNET bridge {args.net:04X}.{args.node:04X} socket {BRIDGE_SOCKET} on UDP port {segment.port}')
    try:
        while True:
            time.sleep(3600)
    except KeyboardInterrupt:
        pass
    finally:
        bridge.close()
        segment.close()


if __name__ == '__main__':
    main()
