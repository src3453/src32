"""/dev/cnet and libcnet on src32_testbench (doc/spec_Foundation_API.md §9.6).

The CNET card (examples/wasm-cnet) sits in expansion slot 0 on a UDP segment.
A Python peer on the other end of the segment speaks the wire format
(doc/spec_NET.md §3, §4, §8.3) on its own, so the tests also check the
card's frames against an independent implementation. The peer is node
0001.0020; Foundation's card is the fixed node 0001.0010.

    python -m unittest discover -s foundation/tests/cnet -v

Needs src32_testbench, Clang, Cargo with the wasm32-unknown-unknown target
(the card is built here) and localhost UDP. FND_MM=nommu also applies.
"""
import io
import os
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import unittest
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE.parent / 'shell'))
import test_shell  # noqa: E402

NET = 0x0001
FOUNDATION_NODE = 0x0010
PEER_NODE = 0x0020
SEGMENT_ID = 5
CARD_WASM = ROOT / 'examples/wasm-cnet/target/wasm32-unknown-unknown/release/cpt32_wasm_cnet_card.wasm'


def crc16(data, crc=0xFFFF):
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) if crc & 0x8000 else crc << 1
            crc &= 0xFFFF
    return crc


def datagram(dst_node, dst_socket, src_socket, proto, payload, dst_net=NET):
    header = bytearray(struct.pack('>BBHHHHHHBB', 0x10, proto, len(payload), 0, dst_net, dst_node,
                                   NET, PEER_NODE, dst_socket, src_socket))
    struct.pack_into('>H', header, 4, crc16(bytes(header) + payload))
    return bytes(header) + payload


def link_frame(dst, src, kind, payload):
    return struct.pack('>HHBBH', dst, src, kind, 0, len(payload)) + payload


class Peer:
    """A CNET node on the far end of the UDP segment."""

    def __init__(self):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(('127.0.0.1', 0))
        self.sock.settimeout(0.05)
        self.port = self.sock.getsockname()[1]
        self.target = None  # the testbench's UDP port
        self.sender = 0x12345678
        self.received = []  # (src_net, src_node, src_socket, dst_socket, proto, payload)
        self.bad = []
        self.push = None  # datagram the peer keeps sending to Foundation
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)

    def send_frame(self, frame):
        packet = b'CNET' + struct.pack('>BBHI', 1, 0, SEGMENT_ID, self.sender) + frame
        self.sock.sendto(packet, ('127.0.0.1', self.target))

    def handle(self, packet):
        if len(packet) < 12 or packet[:4] != b'CNET' or packet[4] != 1:
            self.bad.append(('udp header', packet))
            return
        if struct.unpack('>H', packet[6:8])[0] != SEGMENT_ID:
            self.bad.append(('segment id', packet))
            return
        frame = packet[12:]
        dst, src, kind, _, length = struct.unpack('>HHBBH', frame[:8])
        payload = frame[8:]
        if length != len(payload):
            self.bad.append(('link length', frame))
            return
        if kind == 0x81:  # PROBE: defend our own node only, so 0x0010 is accepted
            if dst == PEER_NODE:
                self.send_frame(link_frame(PEER_NODE, PEER_NODE, 0x82, b''))
            return
        if kind != 0x01 or dst not in (PEER_NODE, 0xFFFF):
            return
        if len(payload) < 16:
            self.bad.append(('datagram header', payload))
            return
        ver_hop, proto, dlen, crc, dnet, dnode, snet, snode, dsock, ssock = struct.unpack(
            '>BBHHHHHHBB', payload[:16])
        body = payload[16:]
        check = bytearray(payload[:16])
        check[0] &= 0xF0
        check[4:6] = b'\0\0'
        if ver_hop >> 4 != 1 or dlen != len(body) or crc16(bytes(check) + body) != crc:
            self.bad.append(('datagram check', payload))
            return
        self.received.append((snet, snode, ssock, dsock, proto, body))
        if dsock == 4 and proto == 0x02 and body[:1] == b'\x01':
            reply = b'\x02' + body[1:]
            self.send_frame(link_frame(snode, PEER_NODE, 0x01, datagram(snode, ssock, 4, 0x02, reply)))

    def run(self):
        while not self.stop.is_set():
            try:
                packet, _ = self.sock.recvfrom(4096)
            except socket.timeout:
                if self.push and self.target:
                    self.send_frame(self.push)
                continue
            except OSError:
                continue
            self.handle(packet)

    def close(self):
        self.stop.set()
        self.thread.join(timeout=2)
        self.sock.close()


def free_udp_port():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


class CnetTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        test_shell.ShellTests.setUpClass.__func__(cls)
        cargo = shutil.which('cargo')
        if cargo:
            subprocess.run([cargo, 'build', '--manifest-path', str(ROOT / 'examples/wasm-cnet/Cargo.toml'),
                            '--release', '--target', 'wasm32-unknown-unknown'],
                           check=True, capture_output=True, timeout=600)
        if not CARD_WASM.is_file():
            raise RuntimeError(f'build the CNET card first: {CARD_WASM}')
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, 'w') as package:
            package.writestr('manifest.yaml', 'abi_version: 1\nmodule: card.wasm\ncapabilities: [network]\n')
            package.writestr('card.wasm', CARD_WASM.read_bytes())
        cls.card = buffer.getvalue()

    @classmethod
    def tearDownClass(cls):
        test_shell.ShellTests.tearDownClass.__func__(cls)

    def setUp(self):
        self.peer = Peer()
        self.peer.target = free_udp_port()
        self.peer.thread.start()

    def tearDown(self):
        self.peer.close()

    def run_shell(self, *batches, card=True, step=60):
        run = Path(tempfile.mkdtemp(dir=self.work))
        (run / 'disk.img').write_bytes(self.disk)
        config = 'memory:\n  ram_mib: 16\n  vram_mib: 4\n'
        config += 'disk:\n  - slot: 0\n    type: "hdd"\n    image: "disk.img"\n    read_only: false\n'
        if card:
            (run / 'cnet.cpt32card').write_bytes(self.card)
            config += (
                'network_segment:\n'
                f'  - name: lan\n    network: {NET}\n'
                f'    udp:\n      bind: "127.0.0.1:{self.peer.target}"\n'
                f'      peers: ["127.0.0.1:{self.peer.port}"]\n      segment_id: {SEGMENT_ID}\n'
                'expansion_card:\n'
                f'  - slot: 0\n    package: cnet.cpt32card\n'
                f'    network: {{ segment: lan, node: {FOUNDATION_NODE} }}\n')
        (run / 'config.yaml').write_text(config, encoding='utf-8')
        cpu = os.environ.get('FOUNDATION_TEST_CPU', 'cached')
        command = [str(self.harness), str(self.image), '42', '--cpu', cpu,
                   '--frames', str(test_shell.FRAMES), '--dump-pcg']
        for i, text in enumerate(batches):
            command += ['--type', f'{test_shell.FIRST_FRAME + step * i}={text}']
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        rows = lines[lines.index('PCG SCREEN BEGIN') + 1:lines.index('PCG SCREEN END')]
        self.assertEqual(self.peer.bad, [])
        return '\n'.join(rows)

    def test_info_reports_the_acquired_address(self):
        text = self.run_shell('cnet\\n')
        self.assertIn('link up, address 0001.0010', text)

    def test_ping_gets_echo_replies_from_the_peer(self):
        text = self.run_shell('cnet ping 1.20 2\\n', 'echo status=$?\\n', step=120)
        self.assertIn('reply from 0001.0020 seq=1', text)
        self.assertIn('reply from 0001.0020 seq=2', text)
        self.assertIn('2/2 replies', text)
        self.assertIn('status=0', text)

    def test_send_reaches_the_peer_with_a_valid_datagram(self):
        text = self.run_shell('cnet send 1.20:64 hello world\\n', 'echo status=$?\\n')
        self.assertIn('status=0', text)
        texts = [r for r in self.peer.received if r[3] == 0x64]
        self.assertEqual(len(texts), 1, self.peer.received)
        net, node, src_socket, _, proto, body = texts[0]
        self.assertEqual((net, node, proto, body), (NET, FOUNDATION_NODE, 0x80, b'hello world'))
        self.assertGreaterEqual(src_socket, 128)

    def test_recv_prints_a_datagram_from_the_peer(self):
        self.peer.push = link_frame(FOUNDATION_NODE, PEER_NODE, 0x01,
                                    datagram(FOUNDATION_NODE, 0x64, 0x99, 0x80, b'hi foundation'))
        text = self.run_shell('cnet recv 64\\n', 'echo status=$?\\n', step=120)
        self.assertIn('from 0001.0020:99: hi foundation', text)
        self.assertIn('status=0', text)

    def test_ping_without_an_answer_times_out(self):
        text = self.run_shell('cnet ping 30\\n', 'echo status=$?\\n', step=360)
        self.assertIn('no reply seq=1', text)
        self.assertIn('0/1 replies', text)
        self.assertNotIn('status=0', text)

    def test_short_forms_address_the_local_network(self):
        # ".NODE" and "NODE" mean NET 0, this network (doc/spec_NET.md §2).
        text = self.run_shell('cnet ping .20\\n', 'cnet ping 20\\n', 'echo status=$?\\n', step=120)
        self.assertEqual(text.count('reply from 0001.0020 seq=1'), 2, text)
        self.assertEqual(text.count('1/1 replies'), 2, text)
        self.assertIn('status=0', text)
        text = self.run_shell('cnet ping 1.\\n', 'cnet ping 1:20\\n', 'echo status=$?\\n')
        self.assertEqual(text.count('usage: cnet ping ADDR'), 2, text)  # ":" is no longer a separator

    def test_endpoint_notation_names_the_socket(self):
        # NET.NODE:SOCK (doc/spec_NET.md §2): the socket follows the address
        # after a colon, in hex like NET and NODE; the short forms of the
        # address apply too. 0 and FF are not sockets.
        text = self.run_shell('cnet send 20:65 one\\n', 'cnet send .20:66 two\\n',
                              'cnet send 1.20 64 three\\n', 'cnet send 1.20:FF x\\n',
                              'cnet send 1.20:0 x\\n', 'echo status=$?\\n')
        got = sorted((r[3], r[5]) for r in self.peer.received if r[4] == 0x80)
        self.assertEqual(got, [(0x65, b'one'), (0x66, b'two')])
        self.assertEqual(text.count('usage: cnet send ADDR:SOCK'), 3, text)

    def test_without_a_card_open_reports_no_media(self):
        text = self.run_shell('cnet\\n', card=False)
        self.assertIn('/dev/cnet', text)
        self.assertNotIn('link up', text)


if __name__ == '__main__':
    unittest.main()
