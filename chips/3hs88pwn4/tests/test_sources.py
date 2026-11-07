"""Source checks for the 3HS88PWN4 model: integer-only, generated tables current."""

import re
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


class SourceTests(unittest.TestCase):
    def test_engine_is_integer_only(self):
        # The C++ model is the RTL reference; floating point must not creep in.
        pattern = re.compile(r"\b(float|double|long double)\b|<cmath>|<math\.h>")
        for path in sorted((ROOT / "include").rglob("*.hpp")) + sorted((ROOT / "src").rglob("*.[ch]pp")):
            for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                code = line.split("//", 1)[0]
                self.assertIsNone(pattern.search(code), f"{path.relative_to(ROOT)}:{number}: {line.strip()}")

    def test_generated_tables_are_current(self):
        result = subprocess.run([sys.executable, str(ROOT / "tools" / "gen_tables.py"), "--check"],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
