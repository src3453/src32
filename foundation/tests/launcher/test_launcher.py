"""Foundation launcher: catalog dependencies, command line, saved state (no display needed)."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


launcher = load("foundation_launcher_under_test", ROOT / "foundation/launcher.py")
boot = load("foundation_launcher_boot", ROOT / "foundation/boot/build.py")
run = load("foundation_launcher_run", ROOT / "foundation/run.py")


class CatalogTests(unittest.TestCase):
    def setUp(self):
        self.catalog = launcher.Catalog.from_build(boot)

    def test_lists_come_from_build_py(self):
        self.assertEqual(self.catalog.commands, boot.COMMANDS)
        self.assertEqual(self.catalog.libs, boot.LIBS)
        self.assertIn("chip8", self.catalog.needs)

    def test_every_module_is_described(self):
        for name in [*boot.COMMANDS, *boot.LIBS]:
            self.assertIn(name, launcher.DESCRIPTIONS)

    def test_dependencies_follow_toggles(self):
        selected = self.catalog.nothing()
        selected = self.catalog.toggle(selected, "commands", "chip8", True)
        self.assertEqual(selected["libs"], {"sgc"})
        selected = self.catalog.toggle(selected, "libs", "sgc", False)
        self.assertNotIn("chip8", selected["commands"])

    def test_default_has_no_selection_arguments(self):
        argv, env = launcher.build_invocation(launcher.default_settings(self.catalog), self.catalog)
        self.assertNotIn("--commands", argv)
        self.assertNotIn("--libs", argv)
        self.assertEqual(env, {})

    def test_command_line(self):
        settings = launcher.default_settings(self.catalog)
        settings.update(commands=["ls", "cat"], libs=[], disk=True, nommu=True, ram_mib="64",
                        disk_blocks="131072", jobs="4", bin_dirs=["/x/doom"], extra_args="--frames 3")
        self.assertEqual(launcher.validate(dict(settings, bin_dirs=[str(ROOT)]), self.catalog), [])
        argv, env = launcher.build_invocation(settings, self.catalog, python="py")
        self.assertEqual(argv[:2], ["py", str(ROOT / "foundation/run.py")])
        self.assertEqual(argv[argv.index("--commands") + 1], "cat,ls")  # canonical order
        self.assertEqual(argv[argv.index("--libs") + 1], "")
        self.assertIn("--disk", argv)
        self.assertEqual(argv[-3:], ["--arg", "--frames", "3"])
        self.assertEqual(env, {"FND_MM": "nommu"})

    def test_lua(self):
        settings = launcher.default_settings(self.catalog)
        settings.update(lua=True, lua_source="/l", newlib_source="/n")
        argv, _ = launcher.build_invocation(settings, self.catalog)
        self.assertEqual(argv[argv.index("--lua-source") + 1], "/l")
        self.assertEqual(argv[argv.index("--newlib-source") + 1], "/n")
        with patch.dict(launcher.os.environ, {}, clear=True):
            self.assertEqual(len(launcher.validate(dict(settings, lua_source="", newlib_source=""), self.catalog)), 2)

    def test_validation(self):
        settings = launcher.default_settings(self.catalog)
        settings.update(commands=["chip8"], libs=[], ram_mib="0", jobs="x", bin_dirs=["/no/such"])
        self.assertEqual(len(launcher.validate(settings, self.catalog)), 4)


class StateTests(unittest.TestCase):
    def test_round_trip_and_new_modules_start_enabled(self):
        catalog = launcher.Catalog(["a", "b"], ["x"], [])
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "sub/state.json"
            settings = launcher.default_settings(catalog)
            settings.update(commands=["a"], ram_mib="32", disk=True)
            launcher.save_state(settings, catalog, path)
            self.assertEqual(launcher.load_state(catalog, path)["commands"], ["a"])
            self.assertEqual(launcher.load_state(catalog, path)["ram_mib"], "32")
            grown = launcher.Catalog(["a", "b", "c"], ["x"], [])
            self.assertEqual(launcher.load_state(grown, path)["commands"], ["a", "c"])  # b stays off, c is new

    def test_bad_state_falls_back_to_defaults(self):
        catalog = launcher.Catalog(["a"], [], [])
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "state.json"
            for text in ("not json", "[]", json.dumps({"ram_mib": 5, "commands": "a"})):
                path.write_text(text)
                self.assertEqual(launcher.load_state(catalog, path), launcher.default_settings(catalog))


class BuildSelectionTests(unittest.TestCase):
    def test_unknown_and_dependency_errors_stop_before_compilation(self):
        with tempfile.TemporaryDirectory() as tmp:
            for option in (["--commands", "nosuch"], ["--libs", "nosuch"], ["--commands", "chip8", "--libs", "rle"]):
                with patch.object(boot, "driver") as compiler, contextlib.redirect_stderr(io.StringIO()):
                    with self.assertRaises(SystemExit) as error:
                        boot.main([str(Path(tmp) / "out"), *option])
                    self.assertEqual(error.exception.code, 2)
                    compiler.assert_not_called()

    def test_run_py_forwards_selection(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "assets").mkdir()
            (root / "assets/UNSCII.F08").write_bytes(bytes(2048))
            argv = ["run.py", "--build-only", "--commands", "ls,cat", "--libs", ""]
            with patch.object(run, "ROOT", root), patch.object(run.sys, "argv", argv), \
                    patch.object(run.subprocess, "run") as sub:
                self.assertEqual(run.main(), 0)
            build = sub.call_args_list[0].args[0]
            self.assertEqual(build[build.index("--commands") + 1], "ls,cat")
            self.assertEqual(build[build.index("--libs") + 1], "")


if __name__ == "__main__":
    unittest.main()
