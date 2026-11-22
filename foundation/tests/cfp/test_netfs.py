"""netfs and /net on src32_testbench (doc/spec_NET_FP.md §11).

Foundation runs with the CNET card on a UDP segment whose only peer is the
CFP server tools/cfp-server/cfp_server.py, run in this process at node
FF02 with a temporary host directory as the export "share". The shell then
mounts it on /net/s and works on it with the usual commands.

    python -m unittest discover -s foundation/tests/cfp -v

Needs what foundation/tests/cnet/test_cnet.py needs (testbench, Clang,
Cargo with wasm32 or a built card, localhost UDP). FND_MM=nommu also
applies.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE.parent / 'cnet'))
sys.path.insert(0, str(ROOT / 'tools/l3p'))
sys.path.insert(0, str(ROOT / 'tools/cfp-server'))
import test_cnet  # noqa: E402
import l3p  # noqa: E402
import cfp_server as cfp  # noqa: E402

test_shell = test_cnet.test_shell
STOP_TICKS = 3000  # 30 s at 100 Hz: the kernel stops the run (about frame 1800)
FRAMES = 1900


class NetfsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        test_cnet.CnetTests.setUpClass.__func__(cls)
        # A longer run than the shell tests' image allows.
        command = [sys.executable, str(ROOT / 'foundation/boot/build.py'), str(cls.work / 'long'),
                   '--stop-ticks', str(STOP_TICKS)]
        clang = os.environ.get('CLANG') or test_cnet.shutil.which('clang')
        if clang:
            command += ['--clang', clang]
        result = subprocess.run(command, capture_output=True, text=True, encoding='utf-8', errors='replace',
                                timeout=900)
        if result.returncode != 0:
            raise RuntimeError(result.stdout + result.stderr)
        cls.image = cls.work / 'long/foundation.wax'
        cls.disk = (cls.work / 'long/foundation.img').read_bytes()

    @classmethod
    def tearDownClass(cls):
        test_shell.ShellTests.tearDownClass.__func__(cls)

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='netfs-')
        self.share = Path(self.tmp.name) / 'share'
        self.share.mkdir()
        (self.share / 'hello.txt').write_bytes(b'hello from the host\n')
        (self.share / 'sub').mkdir()
        (self.share / 'sub' / 'inner.txt').write_bytes(b'inner\n')
        (self.share / 'big.bin').write_bytes(bytes(range(256)) * 40)  # 10240 bytes
        self.ro = Path(self.tmp.name) / 'ro'
        self.ro.mkdir()
        (self.ro / 'note.txt').write_bytes(b'read only\n')
        self.testbench_port = test_cnet.free_udp_port()
        self.segment = l3p.Segment(('127.0.0.1', 0), test_cnet.SEGMENT_ID,
                                   peers=[('127.0.0.1', self.testbench_port)])
        self.node = l3p.Node(self.segment, test_cnet.NET, cfp.CFP_NODE)
        self.server = cfp.CfpServer([cfp.Export('share', cfp.HostDir(self.share)),
                                     cfp.Export('ro', cfp.HostDir(self.ro, read_only=True))])
        self.endpoint = l3p.Endpoint(self.node, cfp.CFP_SOCKET)
        self.endpoint.listen(l3p.KIND_TXN, self.server)
        self.segment.start()

    def tearDown(self):
        self.endpoint.close()
        self.segment.close()
        self.tmp.cleanup()

    def run_shell(self, *commands, start=40, step=60):
        """Types each command `step` frames after the previous one."""
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
        command = [str(self.harness), str(self.image), '42', '--cpu', cpu, '--frames', str(FRAMES), '--dump-pcg']
        frame = start
        for text in commands:
            command += ['--type', f'{frame}={text}\\n']
            frame += step
        self.assertLess(frame, FRAMES)
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=900)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        rows = lines[lines.index('PCG SCREEN BEGIN') + 1:lines.index('PCG SCREEN END')]
        return '\n'.join(row.rstrip() for row in rows)

    def test_mount_list_read_and_exports(self):
        text = self.run_shell('netfs exports', 'netfs mount .FF02 share s', 'netfs', 'ls /net',
                              'ls /net/s', 'cat /net/s/hello.txt', 'cat /net/s/sub/inner.txt',
                              'cat /net/S/sub/../HELLO.TXT', 'echo status=$?')
        self.assertIn('0001.FF02  share', text)
        self.assertIn('0001.FF02  ro ro', text)
        self.assertIn('/net/s: share on 0000.FF02', text)
        self.assertIn('/net/s           0000.FF02  share       up', text)
        self.assertIn('/> ls /net\ns', text)
        for name in ('big.bin', 'hello.txt', 'sub'):
            self.assertIn(name, text)
        self.assertIn('inner', text)
        self.assertEqual(text.count('hello from the host'), 2)
        self.assertIn('status=0', text)

    def test_write_mkdir_copy_move_remove(self):
        text = self.run_shell('netfs mount .FF02 share s', 'echo written > /net/s/new.txt',
                              'echo more >> /net/s/new.txt', 'mkdir /net/s/d',
                              'cp /net/s/big.bin /net/s/d/copy.bin', 'mv /net/s/new.txt /net/s/d/moved.txt',
                              'cp /net/s/hello.txt /tmp/local.txt', 'cat /tmp/local.txt',
                              'rm /net/s/hello.txt', 'rmdir /net/s/sub', 'mv /net/s/d /tmp/d',
                              'echo status=$?')
        self.assertEqual((self.share / 'd' / 'moved.txt').read_bytes(), b'written\nmore\n')
        self.assertEqual((self.share / 'd' / 'copy.bin').read_bytes(), (self.share / 'big.bin').read_bytes())
        self.assertFalse((self.share / 'new.txt').exists())
        self.assertFalse((self.share / 'hello.txt').exists())
        self.assertTrue((self.share / 'sub').is_dir())  # not empty
        self.assertIn('hello from the host', text)
        self.assertIn('rmdir: /net/s/sub: directory not empty', text)
        self.assertIn('cross-device', text.lower())
        self.assertNotIn('status=0', text)  # the last mv failed

    def test_cwd_relative_paths_and_read_only(self):
        text = self.run_shell('netfs mount .FF02 share s', 'netfs mount .FF02 ro r -r', 'cd /net/s/sub',
                              'pwd', 'cat inner.txt', 'cat ../hello.txt', 'ls ..', 'cd ..', 'pwd',
                              'cd ../r', 'cat note.txt', 'echo x > note.txt', 'cd /', 'netfs umount r',
                              'ls /net', 'echo done=$?')
        self.assertIn('/net/s/sub> pwd\n/net/s/sub', text)
        self.assertIn('inner', text)
        self.assertIn('hello from the host', text)
        self.assertIn('/net/s> pwd\n/net/s', text)
        self.assertIn('read only', text)
        self.assertIn('read-only', text.lower())
        self.assertIn('/> ls /net\ns\n', text)
        self.assertIn('done=0', text)
        self.assertEqual((self.ro / 'note.txt').read_bytes(), b'read only\n')

    def test_mount_again_after_a_reboot(self):
        # The server still holds the first boot's association from the same
        # node and socket; libl3p must not repeat its NONCE (spec_NET_L3P
        # §4.1), or the server answers with that old association.
        for _ in range(2):
            text = self.run_shell('netfs mount .FF02 share s', 'cat /net/s/hello.txt')
            self.assertIn('/net/s: share on 0000.FF02', text)
            self.assertIn('hello from the host', text)

    def test_errors(self):
        text = self.run_shell('netfs mount .FF02 nothere x', 'netfs mount .FF02 share s',
                              'netfs mount .FF02 share s', 'cat /net/s/missing', 'cat /net/none/x',
                              'mkdir /net/new', 'netfs umount nope', 'echo done=$?')
        self.assertIn('netfs: x: not found', text)
        self.assertIn('netfs: s: already exists', text)
        self.assertIn('cat: /net/s/missing: not found', text)
        self.assertIn('cat: /net/none/x: not found', text)
        self.assertIn('mkdir: /net/new: read-only', text)
        self.assertIn('netfs: nope: not found', text)


if __name__ == '__main__':
    unittest.main()
