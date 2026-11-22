"""The two-instance chat demo (examples/cnet-chat/run_demo.py --headless).

Two src32_testbench processes, each a Foundation with a CNET card, share a
localhost UDP segment; the demo types a conversation into both and checks
that each screen shows the other's lines.

    python -m unittest discover -s foundation/tests/cnet -p test_demo.py -v
"""
import os
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]


class ChatDemoTests(unittest.TestCase):
    def test_two_instances_exchange_chat_lines(self):
        command = [sys.executable, str(ROOT / 'examples/cnet-chat/run_demo.py'), '--headless']
        if os.environ.get('CLANG'):
            command += ['--clang', os.environ['CLANG']]
        result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, encoding='utf-8',
                                errors='replace', timeout=1200)
        self.assertEqual(result.returncode, 0, result.stdout[-4000:] + result.stderr[-4000:])
        self.assertIn("both sides received the other's messages", result.stdout)
        self.assertIn('CNET chat: 0001.0010:77 -> 0001.0020:77', result.stdout)
        self.assertIn('CNET chat: 0001.0020:77 -> 0001.0010:77', result.stdout)


if __name__ == '__main__':
    unittest.main()
