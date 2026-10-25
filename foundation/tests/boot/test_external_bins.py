"""External WAX overlays: actual FS2 contents, validation and launcher options."""
import contextlib
import importlib.util
import io
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


boot = load('external_bins_boot', ROOT / 'foundation/boot/build.py')
launcher = load('external_bins_run', ROOT / 'foundation/run.py')


class ExternalBinsTests(unittest.TestCase):
    def test_overlay_is_executable_and_keeps_source_snapshot(self):
        with tempfile.TemporaryDirectory() as tmp:
            first, second = Path(tmp) / 'first', Path(tmp) / 'second'
            first.mkdir(); second.mkdir()
            (first / 'game.wax').write_bytes(b'old game')
            (first / 'tool.WAX').write_bytes(b'external tool')
            (first / 'notes.txt').write_text('not a command')
            (first / 'fnd.1.wal').write_bytes(b'not a command')
            (first / 'nested').mkdir()
            (first / 'nested/hidden.wax').write_bytes(b'not recursive')
            (second / 'GAME.WAX').write_bytes(b'new game')
            bins = boot.external_bins([first, second])
            (second / 'GAME.WAX').write_bytes(b'overwritten during build')
            files = {'/bin/game.wax': (b'from --root', None), '/data.txt': (b'data', None)}
            boot.overlay_files(files, bins)
            image = boot.disk_image(bytes(510) + b'\x55\xaa', b'boot', files, 8192)
            volume = boot.fs2.Volume(image)
            game = volume.resolve('/bin/game.wax')
            self.assertEqual(volume.read(game), b'new game')
            self.assertEqual(game['mode'], 0x3D)
            self.assertEqual(volume.read(volume.resolve('/bin/tool.WAX')), b'external tool')
            self.assertEqual(set(files), {'/bin/GAME.WAX', '/bin/tool.WAX', '/data.txt'})
            self.assertEqual(boot.fs2.check_image(image), [])

    def test_bad_directory_stops_before_compilation(self):
        with tempfile.TemporaryDirectory() as tmp:
            for directory in (Path(tmp), Path(tmp) / 'missing'):
                with patch.object(boot, 'driver') as compiler, contextlib.redirect_stderr(io.StringIO()):
                    with self.assertRaises(SystemExit) as error:
                        boot.main([str(Path(tmp) / 'out'), '--bin-dir', str(directory)])
                    self.assertEqual(error.exception.code, 2)
                    compiler.assert_not_called()

    def test_launcher_forwards_all_directories_and_capacity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'assets').mkdir()
            (root / 'assets/UNSCII.F08').write_bytes(bytes(2048))
            directories = [root / 'doom', root / 'busybox']
            argv = ['run.py', '--build-only', '-j', '2', '--ram-mib', '64', '--disk-blocks', '131072']
            for directory in directories:
                argv += ['--bin-dir', str(directory)]
            with patch.object(launcher, 'ROOT', root), patch.object(sys, 'argv', argv), \
                    patch.object(launcher.subprocess, 'run') as run:
                self.assertEqual(launcher.main(), 0)
            build = run.call_args_list[0].args[0]
            indices = [i for i, arg in enumerate(build) if arg == '--bin-dir']
            self.assertEqual([build[i + 1] for i in indices], list(map(str, directories)))
            self.assertEqual(build[build.index('--disk-blocks') + 1], '131072')
            config = (root / 'target/foundation-boot/run/config.yaml').read_text()
            self.assertIn('ram_mib: 64', config)
            self.assertEqual(run.call_count, 2)  # boot build and cargo; no GUI


if __name__ == '__main__':
    unittest.main()
