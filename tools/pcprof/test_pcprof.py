import io
import contextlib
import unittest
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import pcprof


def label(name):
    return "llvm_" + name.encode().hex()


ASM = f""".ORG 0x1000
{label("foo")}:
    ADDI R1, R0, 1
__llvm_src32_block_1:
    ADDI R1, R1, 1
{label("bar")}:
    ADDI R2, R0, 2
    JR R31
{label(".str.1")}:
    .DB 65, 0
"""

PROFILE = """K 00001000 10 12
K 00001004 5 5
K 00001008 3 4
U 00200000 7 9
"""


class PcProfTests(unittest.TestCase):
    def test_symbols_skip_block_labels_and_data(self):
        symbols = pcprof.function_symbols(ASM)
        self.assertEqual([name for _, name in symbols], ["foo", "bar"])
        self.assertEqual([address for address, _ in symbols], [0x1000, 0x1008])

    def test_summary_attributes_pcs_to_functions(self):
        summary = pcprof.summarize(PROFILE, pcprof.Symbols(pcprof.function_symbols(ASM)))
        by_name = {row["function"]: row for row in summary["functions"]}
        self.assertEqual(by_name["foo"]["instructions"], 15)   # the block label does not split it
        self.assertEqual(by_name["foo"]["calls"], 10)          # hits of the entry PC
        self.assertEqual(by_name["foo"]["instructions_per_call"], 1.5)
        self.assertEqual(by_name["bar"]["instructions"], 3)
        self.assertAlmostEqual(by_name["foo"]["share"], 15 / 18)
        self.assertEqual(summary["totals"], {"kernel": [18, 21], "user": [7, 9]})
        self.assertEqual(summary["user_regions"][0]["base"], 0x200000)

    def test_report_prints_a_table(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            pcprof.print_summary(pcprof.summarize(PROFILE, pcprof.Symbols(pcprof.function_symbols(ASM))))
        text = out.getvalue()
        self.assertIn("foo", text)
        self.assertIn("kernel 18", text)


if __name__ == "__main__":
    unittest.main()
