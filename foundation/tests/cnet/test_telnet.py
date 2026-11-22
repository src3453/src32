"""Foundation's telnet / ssh through the CNET TCP bridge (doc/spec_NET_bridge.md).

src32_testbench runs Foundation with the CNET card on a UDP segment whose
only peer is tools/cnet-bridge/cnet_bridge.py, run in this process. The
bridge connects to a TCP server in this process: a small telnet server, or
a paramiko SSH server (those tests skip without paramiko).

    python -m unittest discover -s foundation/tests/cnet -p test_telnet.py -v

Needs what test_cnet.py needs (testbench, Clang, Cargo with wasm32, localhost
UDP). FND_MM=nommu also applies.
"""
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / 'tools/cnet-bridge'))
import test_cnet  # noqa: E402
import cnet_bridge as cb  # noqa: E402
import test_cnet_bridge  # noqa: E402

test_shell = test_cnet.test_shell
IAC, WILL, WONT, DO, DONT, SB, SE = 255, 251, 252, 253, 254, 250, 240


class TelnetServer(test_cnet_bridge.TcpServer):
    """Asks for ECHO, SGA, NAWS and TTYPE like a telnetd, echoes what is
    typed, answers each line with "you said: LINE" and closes after "bye"."""

    def __init__(self):
        self.raw = bytearray()
        super().__init__(TelnetServer.serve)

    def serve(self, conn):
        conn.sendall(bytes([IAC, WILL, 1, IAC, WILL, 3, IAC, DO, 31, IAC, DO, 24]) + b'welcome\r\n$ ')
        line = bytearray()
        state = 0
        while True:
            try:
                data = conn.recv(1024)
            except OSError:
                break
            if not data:
                break
            self.raw += data
            for byte in data:
                if state == 1:  # after IAC
                    state = 2 if byte in (WILL, WONT, DO, DONT) else 3 if byte == SB else 0
                    if byte == IAC:
                        state = 0
                    continue
                if state == 2:
                    if byte == 24:  # WILL TTYPE: ask for the name
                        conn.sendall(bytes([IAC, SB, 24, 1, IAC, SE]))
                    state = 0
                    continue
                if state == 3:
                    if byte == SE:
                        state = 0
                    continue
                if byte == IAC:
                    state = 1
                elif byte == 13:
                    text = line.decode('utf-8', 'replace')
                    line.clear()
                    if text == 'bye':
                        conn.sendall(b'\r\nsee you\r\n')
                        conn.close()
                        return
                    conn.sendall(f'\r\nyou said: {text}\r\n$ '.encode())
                elif byte in (0, 10):
                    pass
                elif byte == 0x7F:
                    if line:
                        line.pop()
                        conn.sendall(b'\b \b')
                else:
                    line.append(byte)
                    conn.sendall(bytes([byte]))
        conn.close()


class TelnetTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        test_cnet.CnetTests.setUpClass.__func__(cls)

    @classmethod
    def tearDownClass(cls):
        test_shell.ShellTests.tearDownClass.__func__(cls)

    def setUp(self):
        self.testbench_port = test_cnet.free_udp_port()
        self.segment = cb.Segment(('127.0.0.1', 0), test_cnet.SEGMENT_ID,
                                  peers=[('127.0.0.1', self.testbench_port)])
        self.node = cb.Node(self.segment, test_cnet.NET, cb.BRIDGE_NODE)
        self.tmp = tempfile.TemporaryDirectory()
        self.bridge = cb.Bridge(self.node, ['127.0.0.1'], known_hosts=Path(self.tmp.name) / 'known_hosts')
        self.segment.start()
        self.bridge.start()

    def tearDown(self):
        self.bridge.close()
        self.segment.close()
        self.tmp.cleanup()

    def run_shell(self, *batches):
        """batches are (frame, text) pairs."""
        run = Path(tempfile.mkdtemp(dir=self.work))
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'cnet.cpt32card').write_bytes(self.card)
        config = ('memory:\n  ram_mib: 16\n  vram_mib: 4\n'
                  'disk:\n  - slot: 0\n    type: "hdd"\n    image: "disk.img"\n    read_only: false\n'
                  'network_segment:\n'
                  f'  - name: lan\n    network: {test_cnet.NET}\n'
                  f'    udp:\n      bind: "127.0.0.1:{self.testbench_port}"\n'
                  f'      peers: ["127.0.0.1:{self.segment.port}"]\n      segment_id: {test_cnet.SEGMENT_ID}\n'
                  'expansion_card:\n'
                  '  - slot: 0\n    package: cnet.cpt32card\n'
                  f'    network: {{ segment: lan, node: {test_cnet.FOUNDATION_NODE} }}\n')
        (run / 'config.yaml').write_text(config, encoding='utf-8')
        cpu = os.environ.get('FOUNDATION_TEST_CPU', 'cached')
        command = [str(self.harness), str(self.image), '42', '--cpu', cpu,
                   '--frames', str(test_shell.FRAMES), '--dump-pcg']
        for frame, text in batches:
            command += ['--type', f'{frame}={text}']
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        rows = lines[lines.index('PCG SCREEN BEGIN') + 1:lines.index('PCG SCREEN END')]
        return '\n'.join(rows)

    def test_telnet_session_through_the_bridge(self):
        server = TelnetServer()
        self.addCleanup(server.close)
        text = self.run_shell((30, f'telnet 127.0.0.1 {server.port}\\n'), (150, 'hello\\n'),
                              (270, 'bye\\n'), (450, 'echo status=$?\\n'))
        self.assertIn(f'Trying telnet://127.0.0.1:{server.port} via 0000:FF01...', text)
        self.assertIn('Connected. Escape character is Ctrl+]', text)
        self.assertIn('welcome', text)
        self.assertIn('$ hello', text)  # echoed by the server (it said WILL ECHO), not twice
        self.assertNotIn('hhello', text)
        self.assertIn('you said: hello', text)
        self.assertIn('see you', text)
        self.assertIn('Connection closed by foreign host.', text)
        self.assertIn('status=0', text)
        raw = bytes(server.raw)
        self.assertIn(bytes([IAC, DO, 1]), raw)                       # DO ECHO
        self.assertIn(bytes([IAC, WILL, 31, IAC, SB, 31, 0, 80, 0, 30, IAC, SE]), raw)  # NAWS 80x30
        self.assertIn(bytes([IAC, SB, 24, 0]) + b'ANSI' + bytes([IAC, SE]), raw)       # TTYPE IS ANSI
        self.assertIn(b'hello\r\0', raw)                              # Enter is CR NUL

    def test_refused_target_reports_the_bridge_reason(self):
        text = self.run_shell((30, 'telnet 10.1.2.3\\n'), (240, 'echo status=$?\\n'))
        self.assertIn('telnet: 10.1.2.3:23: not allowed by the bridge', text)
        self.assertNotIn('status=0', text)

    def test_bridge_address_option(self):
        # Nobody answers at FF02; the 15 s OPEN timeout is longer than the run.
        text = self.run_shell((30, 'telnet -b :FF02 127.0.0.1\\n'))
        self.assertIn('Trying telnet://127.0.0.1:23 via 0000:FF02...', text)

    @unittest.skipUnless(test_cnet_bridge.paramiko, 'paramiko is not installed')
    def test_ssh_login_through_the_bridge(self):
        server = test_cnet_bridge.SshServer()
        self.addCleanup(server.close)
        text = self.run_shell((30, f'ssh -p {server.port} cpt@127.0.0.1\\n'), (150, 'yes\\n'),
                              (230, 'secret\\n'), (300, 'ls\\n'), (330, 'exit\\n'),
                              (450, 'echo status=$?\\n'))  # the kernel stops at about frame 480
        self.assertIn(f'Trying ssh://cpt@127.0.0.1:{server.port} via 0000:FF01...', text)
        self.assertIn("The authenticity of host", text)
        self.assertIn("cpt@127.0.0.1's password:", text)
        self.assertNotIn('secret', text)
        self.assertIn('$ ls', text)
        self.assertIn('bye', text)
        self.assertIn('Connection closed by foreign host.', text)
        self.assertIn('status=0', text)
        self.assertEqual(server.pty[0], ('ansi', 80, 30))
        self.assertEqual(bytes(server.typed), b'ls\rexit\r')


if __name__ == '__main__':
    unittest.main()
