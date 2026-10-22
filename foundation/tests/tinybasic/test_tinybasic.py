"""Foundation C TinyBASIC: actual shell -> WAX/WAL -> console/fs IPC.

Uses the shell harness to build a fresh disk and inspect the real PCG output.
Run with FND_MM=slot as well to check private storage in the fixed-slot backend.
"""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'shell'))
import test_shell


class TinyBasicTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        test_shell.ShellTests.setUpClass.__func__(cls)
        volume = test_shell.fs2.Volume(bytearray(cls.disk))
        volume.put('/oversize.bas', b' ' * (0x20000 + 1))
        volume.put('/hello.bas', b'10 input a\n20 print a*2\n30 end\n')
        volume.put('/bad.bas', b'10 goto 999\n')
        cls.disk = bytes(volume.img)

    @classmethod
    def tearDownClass(cls):
        test_shell.ShellTests.tearDownClass.__func__(cls)

    screen = test_shell.ShellTests.screen

    @staticmethod
    def free_kib(rows):
        return rows

    def test_repl_control_flow_and_return(self):
        rows = self.screen('tinybasic\\n', 'cls\\n',
                           'print 2^10, #6 -42\\n',
                           '10 for i=1 to 3\\n', '20 gosub 100\\n', '30 next i\\n',
                           '40 end\\n', '100 print i;\\n', '110 return\\n',
                           'run\\n', 'bye\\n', 'echo returned\\n', step=12)
        text = '\n'.join(rows)
        self.assertIn('1024', text)
        self.assertIn('   -42', text)
        self.assertIn('123OK', text)
        self.assertIn('/> echo returned\nreturned\n/>', text)
        self.assertNotIn('? ERROR', text)

    def test_files_and_error_preserve_program(self):
        rows = self.screen('tinybasic\\n', 'cls\\n',
                           '10 print 7654\\n', 'save "saved.bas"\\n', 'new\\n',
                           'load "saved.bas"\\n', 'load "missing.bas"\\n',
                           'run\\n', 'quit\\n', 'echo done\\n', step=12)
        text = '\n'.join(rows)
        self.assertIn('SAVED', text)
        self.assertIn('LOADED', text)
        self.assertIn('FILE ERROR: not found', text)
        self.assertIn('> run\n7654\nOK', text)
        self.assertIn('/> echo done\ndone', text)
        # Inspect disk bytes too: SAVE writes a text file, with no sector padding.
        volume = test_shell.fs2.Volume((self.last_run / 'disk.img').read_bytes(), allow_dirty=True)
        self.assertEqual(volume.read(volume.resolve('/saved.bas')), b'10 print 7654\r\n')

    def test_redirected_input_and_input_statement(self):
        # A file supplies both the REPL and INPUT; EOF exits without HALT.
        rows = self.screen("echo 'input a' > session.bas\\n",
                           "echo '37' >> session.bas\\n",
                           "echo 'print a+5' >> session.bas\\n",
                           'tinybasic < session.bas > output.txt\\n',
                           'echo status=$?\\n', step=16)
        self.assertIn('status=0', '\n'.join(rows))
        volume = test_shell.fs2.Volume((self.last_run / 'disk.img').read_bytes(), allow_dirty=True)
        self.assertIn(b'42\n', volume.read(volume.resolve('/output.txt')))

    def test_file_argument_and_exit_status(self):
        rows = self.screen('tinybasic hello.bas\\n', '21\\n', 'echo status=$?\\n',
                           'tinybasic bad.bas\\n', 'echo error=$?\\n',
                           'tinybasic missing.bas\\n', 'echo missing=$?\\n', step=16)
        text = '\n'.join(rows)
        self.assertIn('42', text)
        self.assertIn('status=0', text)
        self.assertIn('? ERROR IN LINE 10', text)
        self.assertIn('error=-2', text)
        self.assertIn('missing=-10', text)
        self.assertNotIn('LOADED', text)

    def test_oversize_load_preserves_program(self):
        text = '\n'.join(self.screen('tinybasic\\n', 'cls\\n', '10 print 1234\\n',
                                     'load "oversize.bas"\\n', 'run\\n', 'bye\\n', step=12))
        self.assertIn('FILE ERROR:', text)
        self.assertIn('> run\n1234\nOK', text)

    def test_error_line_and_restart_isolation(self):
        text = '\n'.join(self.screen('tinybasic\\n', 'cls\\n', '10 goto 999\\n', 'run\\n', 'bye\\n', step=12))
        self.assertIn('? ERROR IN LINE 10', text)
        rows = self.screen('tinybasic\\n', 'cls\\n', '10 goto 999\\n', 'run\\n',
                           'bye\\n', 'tinybasic\\n', 'list\\n', 'print a\\n',
                           'bye\\n', 'echo clean\\n', step=12)
        text = '\n'.join(rows)
        # The second launch clears the console; its empty program and A=0
        # demonstrate fresh process memory and reusable heap capacity.
        self.assertIn('> list\nOK', text)
        self.assertIn('> print a\n0\nOK', text)
        self.assertIn('/> echo clean\nclean', text)


if __name__ == '__main__':
    unittest.main()
