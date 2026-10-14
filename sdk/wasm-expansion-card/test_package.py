import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path


SDK_DIR = Path(__file__).resolve().parent
REPO_ROOT = SDK_DIR.parents[1]
BUILDER = SDK_DIR / "package.py"


class PackageBuilderTests(unittest.TestCase):
    def setUp(self):
        self.tempdir = tempfile.TemporaryDirectory()
        self.root = Path(self.tempdir.name)
        self.module = self.root / "card.wasm"
        self.module.write_bytes(b"\0asm\x01\0\0\0")

    def tearDown(self):
        self.tempdir.cleanup()

    def run_builder(self, *arguments):
        return subprocess.run(
            [sys.executable, str(BUILDER), *map(str, arguments)],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_manifest_ids_and_archive_contents_match_asset_order(self):
        first = self.root / "first.bin"
        second = self.root / "second.bin"
        first.write_bytes(b"first")
        second.write_bytes(b"second")
        output = self.root / "plugins" / "timer.cpt32card"

        result = self.run_builder(
            "--asset",
            f"assets/first.bin={first}",
            "--asset",
            f"assets/second.bin={second}",
            self.module,
            output,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        with zipfile.ZipFile(output) as package:
            self.assertEqual(
                package.namelist(),
                ["manifest.yaml", "wasm/card.wasm", "assets/first.bin", "assets/second.bin"],
            )
            self.assertEqual(
                package.read("manifest.yaml").decode(),
                'abi_version: 1\nmodule: "wasm/card.wasm"\nasset:\n'
                '  - id: 0\n    path: "assets/first.bin"\n'
                '  - id: 1\n    path: "assets/second.bin"\n',
            )
            self.assertEqual(package.read("wasm/card.wasm"), self.module.read_bytes())
            self.assertEqual(package.read("assets/first.bin"), b"first")
            self.assertEqual(package.read("assets/second.bin"), b"second")

    def test_unsafe_asset_path_is_rejected_before_creating_package(self):
        asset = self.root / "asset.bin"
        asset.write_bytes(b"data")
        output = self.root / "bad.cpt32card"

        result = self.run_builder(
            "--asset", f"../outside.bin={asset}", self.module, output
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(output.exists())

    def test_output_cannot_overwrite_module_or_asset(self):
        asset = self.root / "asset.bin"
        asset.write_bytes(b"data")
        original_module = self.module.read_bytes()

        module_result = self.run_builder(self.module, self.module)
        asset_result = self.run_builder(
            "--asset", f"assets/data.bin={asset}", self.module, asset
        )

        self.assertNotEqual(module_result.returncode, 0)
        self.assertNotEqual(asset_result.returncode, 0)
        self.assertEqual(self.module.read_bytes(), original_module)
        self.assertEqual(asset.read_bytes(), b"data")


if __name__ == "__main__":
    unittest.main()
