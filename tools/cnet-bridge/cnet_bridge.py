#!/usr/bin/env python3
"""CNET TCP bridge: telnet <-> telnet and telnet <-> SSH (doc/spec_NET_bridge.md).

The bridge is one node on a CNET UDP segment (doc/spec_NET.md §8.3). A CPT32
client (Foundation's telnet / ssh) opens an L3P stream (doc/spec_NET_L3P.md
§6, tools/l3p/l3p.py) to socket 23, sends the target as the first line and
waits for the bridge's "OK" line:

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

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'l3p'))
import l3p  # noqa: E402
from l3p import Segment, Node, parse_hostport, log  # noqa: E402,F401  (re-exported for callers)

BRIDGE_SOCKET = 23
BRIDGE_NODE = 0xFF01
MAX_TARGET = 255          # bytes of the target line, without the LF
TARGET_WAIT = 30.0        # seconds for the client to send the target line


class Aborted(Exception):
    pass


class Stream:
    """The bridge's end of one L3P stream (doc/spec_NET_L3P.md §6), with the
    interface the backends use: write(), close_write(), read() (b'' at EOF,
    queue.Empty after a timeout, Aborted when the association fails)."""

    def __init__(self, assoc):
        self.assoc = assoc

    def write(self, data):
        try:
            self.assoc.write(data)
        except l3p.L3PError as error:
            raise Aborted(str(error))

    def close_write(self):
        self.assoc.close_write()

    def read(self, timeout=None):
        try:
            return self.assoc.read(timeout)
        except TimeoutError:
            raise queue.Empty
        except l3p.L3PError as error:
            raise Aborted(str(error))

    def abort(self, reason, refused=False):
        self.assoc.abort(reason, l3p.R_REFUSED if refused else l3p.R_ABORT)

    @property
    def finished(self):
        return self.assoc.finished


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
    def __init__(self, bridge, assoc, target, pending=b''):
        self.bridge = bridge
        self.assoc = assoc
        self.target = target
        self.stream = Stream(assoc)
        self.pending = pending        # bytes the client sent after the target line
        self.name = assoc.name

    def accept(self):
        """§3.2: the answer line that the connection to the target is up."""
        self.stream.write(b'OK\n')

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
        if self.pending:
            tcp.sendall(self.pending)
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
        if self.pending:
            dialog.pending += telnet.feed(self.pending)
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
    """Accepts L3P streams on socket 23 (doc/spec_NET_bridge.md §3). The
    client's first line names the target; the bridge answers OK when the
    connection is up, or resets the association with a message."""

    def __init__(self, node, allow=(), allow_any=False, max_sessions=8, known_hosts=None, identities=()):
        self.node = node
        self.allow = list(allow)
        self.allow_any = allow_any
        self.known_hosts = Path(known_hosts) if known_hosts else default_known_hosts()
        self.identities = list(identities)
        self.endpoint = l3p.Endpoint(node, BRIDGE_SOCKET, max_assocs=max_sessions)
        self.endpoint.listen(l3p.KIND_STREAM, self)

    def start(self):
        pass

    def close(self):
        self.endpoint.close()

    def on_open(self, assoc):
        threading.Thread(target=self.serve, args=(assoc,), name=f'bridge {assoc.name}', daemon=True).start()

    def serve(self, assoc):
        stream = Stream(assoc)
        line = bytearray()
        deadline = time.monotonic() + TARGET_WAIT
        try:
            while b'\n' not in line:
                left = deadline - time.monotonic()
                if left <= 0 or len(line) > MAX_TARGET + 1:
                    stream.abort('expected the target as the first line', refused=True)
                    return
                try:
                    data = stream.read(timeout=left)
                except queue.Empty:
                    continue
                if not data:
                    stream.abort('expected the target as the first line', refused=True)
                    return
                line += data
        except Aborted:
            return
        text, _, pending = bytes(line).partition(b'\n')
        try:
            target = parse_target(text.decode('utf-8').rstrip('\r'))
        except (UnicodeDecodeError, ValueError) as error:
            stream.abort(f'bad target: {error}', refused=True)
            return
        scheme, host, port, _ = target
        if not (self.allow_any or allowed(self.allow, host, port)):
            log(assoc.name, f'refused {scheme}://{host}:{port} (not allowed)')
            stream.abort(f'{host}:{port}: not allowed by the bridge', refused=True)
            return
        session = (SshSession if scheme == 'ssh' else TelnetSession)(self, assoc, target, pending)
        log(session.name, f'open {scheme}://{host}:{port}')
        session.run_safely()


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
