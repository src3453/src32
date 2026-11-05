"""Lua 5.4（外部ソース）を pkcl と Newlib で組み、Foundation 上でスクリプトと REPL を動かす。

Lua は MIT ライセンスだがリポジトリには含めない（Newlib と同じ）。

    SRC32_LUA_SOURCE=/path/to/lua-5.4.9 SRC32_NEWLIB_SOURCE=/path/to/newlib-4.4.0.20231231 \\
        python -m unittest discover -s foundation/tests/lua -v

SRC32_LUA_WAX に組み済みの lua.wax を渡すとビルドを省く。CLANG と SRC32_TESTBENCH で実行ファイルを指定できる。
"""
import os
import re
from pathlib import Path
import subprocess
import sys
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE.parent / 'shell'))
import test_shell  # noqa: E402

LUA = os.environ.get('SRC32_LUA_SOURCE')
NEWLIB = os.environ.get('SRC32_NEWLIB_SOURCE')
PREBUILT = os.environ.get('SRC32_LUA_WAX')

SCRIPT = b"""print(_VERSION)
print(1 + 2, 7 // 2, 7 / 2, 2 ^ 10, 10 % 3)
print(math.pi, math.sqrt(2), math.floor(-3.5), math.maxinteger)
print(string.format('%5.2f|%d|%s|%x', 3.14159, 42, 'str', 255))
print(('abc'):upper(), #'hello', ('x'):rep(3), ('a,b'):find(','))
local t = {}
for i = 1, 5 do t[#t + 1] = i * i end
table.sort(t, function(a, b) return a > b end)
print(table.concat(t, ' '))
print(pcall(error, 'boom'))
print(select(2, pcall(function() local x = nil; return x.y end)))
local co = coroutine.wrap(function() for i = 1, 3 do coroutine.yield(i) end end)
print(co(), co(), co())
print(tostring(1e15), tostring(2^63), 0.1 + 0.2, 1 // 0.0)
print(utf8.char(0x3042), utf8.len('\\xe3\\x81\\x82'))
local f = assert(io.open('/tmp/lua_t.txt', 'w'))
f:write('line1\\nline2\\n'); f:close()
for l in io.lines('/tmp/lua_t.txt') do io.write('[', l, ']') end
print()
print(os.time() > 0, type(os.clock()))
print('done-lua')
"""


@unittest.skipUnless(PREBUILT or (LUA and NEWLIB), 'SRC32_LUA_SOURCE と SRC32_NEWLIB_SOURCE が必要')
class LuaTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        test_shell.STOP_TICKS = 6000
        test_shell.ShellTests.setUpClass.__func__(cls)
        if PREBUILT:
            wax = Path(PREBUILT).read_bytes()
        else:
            out = cls.work / 'lua'
            command = [sys.executable, str(ROOT / 'tools/lua/build.py'), '--lua-source', LUA,
                       '--newlib-source', NEWLIB, '--fnd-wal', str(cls.work / 'root/lib/fnd.1.wal'),
                       '--out', str(out)]
            if os.environ.get('CLANG'):
                command += ['--clang', os.environ['CLANG']]
            result = subprocess.run(command, capture_output=True, text=True, timeout=3000)
            if result.returncode:
                raise RuntimeError(result.stdout + result.stderr)
            wax = (out / 'lua.wax').read_bytes()
        # spawn の staging は 3 MiB（libfnd.wal を含む）。
        assert len(wax) < 0x2C0000, f'lua.wax is {len(wax)} bytes'
        volume = test_shell.fs2.Volume(bytearray(cls.disk))
        volume.put('/bin/lua.wax', wax, mode=0x3D)
        volume.put('/t.lua', SCRIPT, mode=0x3D)
        cls.disk = bytes(volume.img)

    @classmethod
    def tearDownClass(cls):
        test_shell.ShellTests.tearDownClass.__func__(cls)

    def run_commands(self, commands, frames, spacing=70):
        run = self.work / 'run'
        run.mkdir(exist_ok=True)
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'config.yaml').write_text(
            'memory:\n  ram_mib: 16\n  vram_mib: 4\n'
            'disk:\n  - slot: 0\n    type: hdd\n    image: disk.img\n    read_only: false\n', encoding='utf-8')
        command = [str(self.harness), str(self.image), '42', '--frames', str(frames), '--dump-pcg',
                   '--cpu', os.environ.get('FOUNDATION_TEST_CPU', 'cached')]
        for index, text in enumerate(commands):
            command += ['--type', f'{30 + index * spacing}={text}\\n']
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=1800)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout

    def test_script(self):
        screen = self.run_commands(['lua /t.lua'], 4000)
        screen = re.sub(r'[ \t]+', ' ', screen)  # PCG のタブ展開は幅が揃うだけ
        for expected in ('Lua 5.4', '3 3 3.5 1024.0 1', '3.1415926535898', '1.4142135623731', '-4',
                         '9223372036854775807', '3.14|42|str|ff', 'ABC 5 xxx 2 2', '25 16 9 4 1',
                         'false boom', 'attempt to index a nil value', '1 2 3', '1e+15', '9.2233720368548e+18',
                         '0.3', 'inf', '[line1][line2]', 'true number', 'done-lua'):
            self.assertIn(expected, screen)

    def test_repl(self):
        screen = self.run_commands(['lua', 'print(6 * 7)', 'x = 10', 'return x + 0.5', '= x * 2',
                                    'print(', '1)', 'os.exit(0)'], 4000, spacing=150)
        for expected in ('Lua 5.4', '42', '10.5', '20', '1'):
            self.assertIn(expected, screen)


if __name__ == '__main__':
    unittest.main()
