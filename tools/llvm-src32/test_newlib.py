"""External Newlib regressions. Set SRC32_NEWLIB_SOURCE; never downloads sources."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from driver import find_clang

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


class NewlibTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="newlib source test ")
        self.addCleanup(self.temp.cleanup)
        self.work = Path(self.temp.name)
        self.clang = find_clang(None)

    def driver(self, *args, success=True):
        result = subprocess.run([sys.executable, str(HERE / "driver.py"),
                                 "--clang", self.clang, *map(str, args)],
                                cwd=self.work, capture_output=True, text=True,
                                encoding="utf-8", errors="replace", timeout=120)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        return result

    def test_missing_source_and_heap_diagnostics(self):
        source = self.work / "main.c"
        source.write_text("int main(void) { return 42; }", encoding="utf-8")
        result = self.driver(source, "--newlib-source", self.work / "absent", success=False)
        self.assertIn("invalid --newlib-source", result.stderr)
        result = self.driver(source, "--newlib-heap-size", "32", success=False)
        self.assertIn("requires --newlib-source", result.stderr)

    def external(self):
        source = os.environ.get("SRC32_NEWLIB_SOURCE")
        if not source:
            self.skipTest("set SRC32_NEWLIB_SOURCE to separately obtained Newlib sources")
        return Path(source).resolve()

    def test_external_newlib_guest(self):
        source = self.external()
        harness = Path(os.environ.get("SRC32_TESTBENCH", ROOT / "target/debug" /
                                     ("src32_testbench.exe" if os.name == "nt" else "src32_testbench"))).resolve()
        self.assertTrue(harness.is_file(), "build src32_testbench first")
        for opt in ("-O0", "-O1", "-O2"):
            with self.subTest(optimization=opt):
                image, depfile = self.work / "demo.wax", self.work / "demo.d"
                self.driver(ROOT / "examples/newlib-c/main.c", "--newlib-source", source,
                            "--depfile", depfile, opt, "-o", image)
                deps = depfile.read_text(encoding="utf-8")
                self.assertIn("nano-mallocr.c", deps)
                self.assertIn("COPYING.NEWLIB", deps)
                self.assertNotIn("newlib-allocator.c", deps)
                for cpu in ("scalar", "pipeline"):
                    result = subprocess.run([str(harness), str(image), "42", "--cpu", cpu],
                                            cwd=self.work, capture_output=True, text=True,
                                            encoding="utf-8", errors="replace", timeout=120)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertIn("PASS:", result.stdout)

    def test_user_string_definition_and_invalid_heap(self):
        source = self.external()
        main = self.work / "custom.c"
        main.write_text("#include <string.h>\nsize_t strlen(const char*s){return 42;}\n"
                        "int main(void){return strlen(\"x\");}\n", encoding="utf-8")
        result = self.driver(main, "--newlib-source", source, "--newlib-heap-size", "7", success=False)
        self.assertIn("multiple of 8", result.stderr)
        ir = self.work / "custom.ll"
        self.driver(main, "--newlib-source", source, "--emit-llvm", "-o", ir)
        text = ir.read_text(encoding="utf-8")
        self.assertEqual(text.count("define dso_local i32 @strlen("), 1)
        self.assertNotIn("@cpt32_newlib_sbrk", text)


if __name__ == "__main__":
    unittest.main()
