"""Host tests of the Foundation WAX loader core (foundation/kernel/wax.c).

Covers the acceptance and rejection cases of doc/spec_WAX_v3.md §8 that do
not need a CPU, plus PROTECTED WAX v2. Execution, protection faults and
process cleanup run on src32_testbench in
tools/llvm-src32/test_toolchain.py (test_foundation_wax_loader).

Run: python -m unittest discover -s foundation/tests/wax -v
CC selects the host C compiler (default: cc, gcc or clang on PATH).
"""
import ctypes
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(ROOT / "tools/wax"))
import wax  # noqa: E402  (v2 packer)
from wax3 import (FUNCTION, OBJECT, PERM_R, PERM_RW, PERM_RX, Block, Dependency,  # noqa: E402
                  Export, Import, build)

(OK, FORMAT, UNSUPPORTED, DEPENDENCY_MISSING, ABI_MISMATCH, DEPENDENCY_CONFLICT,
 DEPENDENCY_CYCLE, SYMBOL_MISSING, SYMBOL_MISMATCH, REGION_LIMIT, RESOURCE_LIMIT) = range(11)

WINDOW = (0x100000, 0x200000)
SIG = "fn(ptr,u32)->u32"


class Result:
    def __init__(self, out, ram, diag_name):
        self.error, self.count, self.entry, self.stack_top, self.stack_size, self.top = out[:6]
        count = out[6]
        self.regions = [tuple(out[7 + 3 * i:10 + 3 * i]) for i in range(count)]
        self.origins = list(out[31:31 + 8])
        self.diag_image, self.diag_index = out[39], out[40]
        self.diag_name = diag_name
        self.ram = ram

    def word(self, address):
        offset = address - WINDOW[0]
        return int.from_bytes(self.ram[offset:offset + 4], "big")


def app(*, base=0x100000, deps=(), imports=(), stack=(0x1FF000, 0x200000), code=None, data=None,
        blocks=None, entry_offset=0, **kw):
    if blocks is None:
        blocks = [Block(base, code if code is not None else bytes(i % 255 + 1 for i in range(0x100)), perm=PERM_RX),
                  Block(base + 0x100, data if data is not None else bytes(0x80), perm=PERM_RW)]
    return build("wax", image_id="app", blocks=blocks, entry_offset=entry_offset,
                 stack_top=stack[1], stack_size=stack[1] - stack[0], deps=deps, imports=imports, **kw)


def lib(library_id, *, minor=0, major=1, exports=None, deps=(), imports=(), align=4, blocks=None, **kw):
    if blocks is None:
        blocks = [Block(0, bytes([0x11]) * 0x80, perm=PERM_RX, alignment=align),
                  Block(0x80, bytes(0x40), perm=PERM_RW)]
    if exports is None:
        exports = [Export("write", FUNCTION, 0x20, 0x10, SIG)]
    return build("wal", image_id=library_id, abi_major=major, abi_minor=minor, blocks=blocks,
                 exports=exports, deps=deps, imports=imports, **kw)


def imp(dep=0, name="write", slot=0x100, kind=FUNCTION, signature=SIG, size=0):
    return Import(dep, name, kind, slot, size, signature if kind == FUNCTION else None)


class LoaderTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cc = os.environ.get("CC") or shutil.which("cc") or shutil.which("gcc") or shutil.which("clang")
        if not cc:
            raise unittest.SkipTest("no host C compiler (set CC)")
        cls.temp = tempfile.TemporaryDirectory(prefix="wax-loader-")
        suffix = ".dll" if os.name == "nt" else ".so"
        library = Path(cls.temp.name) / ("waxhost" + suffix)
        subprocess.run([cc, "-std=c11", "-Wall", "-Wextra", "-Werror", "-O1", "-shared", "-fPIC",
                        "-I", str(ROOT / "foundation/include"), str(ROOT / "foundation/kernel/wax.c"),
                        str(HERE / "host.c"), "-o", str(library)], check=True)
        cls.lib = ctypes.CDLL(str(library))
        cls.lib.host_diag_name.restype = ctypes.c_char_p

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def plan(self, exe, libs=(), window=WINDOW, fill=0xAA):
        data = [bytes(getattr(x, "data", x)) for x in libs]
        exe = bytes(getattr(exe, "data", exe))
        arrays = (ctypes.c_char_p * max(1, len(data)))(*data)
        sizes = (ctypes.c_uint32 * max(1, len(data)))(*map(len, data))
        out = (ctypes.c_uint32 * 41)()
        size = WINDOW[1] - WINDOW[0]
        ram = ctypes.create_string_buffer(bytes([fill]) * size, size)
        self.lib.host_plan(exe, len(exe), arrays, sizes, len(data), window[0], window[1],
                           ram, WINDOW[0], out)
        name = self.lib.host_diag_name()
        return Result(list(out), ram.raw, name.decode() if name else None)

    def ok(self, exe, libs=(), **kw):
        result = self.plan(exe, libs, **kw)
        self.assertEqual(result.error, OK)
        return result

    def rejects(self, error, exe, libs=()):
        self.assertEqual(self.plan(exe, libs).error, error)

    # --- acceptance ---

    def test_dependency_free_wax(self):
        image = app(blocks=[Block(0x100000, bytes(range(1, 0x41)), perm=PERM_RX),
                            Block(0x100040, b"\x01\x02\x03\x04", memory_size=0x40, perm=PERM_RW)],
                    entry_offset=8)
        r = self.ok(image)
        self.assertEqual((r.count, r.entry, r.stack_top, r.stack_size), (1, 0x100008, 0x200000, 0x1000))
        self.assertEqual(r.regions, [(0x1FF000, 0x200000, PERM_RW),
                                     (0x100000, 0x100040, PERM_RX), (0x100040, 0x100080, PERM_RW)])
        self.assertEqual(r.top, 0x100080)
        self.assertEqual(r.word(0x100000), 0x01020304)
        self.assertEqual(r.word(0x100040), 0x01020304)
        self.assertEqual(r.ram[0x44:0x80], bytes(0x3C))           # zero-filled tail (BSS)
        self.assertEqual(r.ram[0xFF000:0x100000], bytes(0x1000))  # stack zeroed
        self.assertEqual(r.ram[0x80:0x84], b"\xAA" * 4)           # holes are not written

    def test_worked_binding_example(self):
        # doc/spec_WAX_v3.md §8, with the console's alignment selecting B = 0x120000.
        console = lib("cpt32.console", minor=3, align=0x20000)
        exe = app(deps=[Dependency("cpt32.console", 1, 2)], imports=[imp(slot=0x100)])
        r = self.ok(exe, [console])
        self.assertEqual(r.origins[:2], [0x100000, 0x120000])
        offset = 0x100100 - WINDOW[0]
        self.assertEqual(r.ram[offset:offset + 8], bytes.fromhex("0012002000120000"))
        self.assertEqual(r.regions, [(0x1FF000, 0x200000, PERM_RW), (0x100000, 0x100100, PERM_RX),
                                     (0x100100, 0x100180, PERM_RW), (0x120000, 0x120080, PERM_RX),
                                     (0x120080, 0x1200C0, PERM_RW)])

    def test_wal_graph_bss_object_and_repeated_import(self):
        base = lib("base", exports=[Export("add", FUNCTION, 0x0, 0x10, "fn(u32,u32)->u32"),
                                    Export("counter", OBJECT, 0x90, 4)],
                   blocks=[Block(0, bytes([0x22]) * 0x80, perm=PERM_RX),
                           Block(0x80, bytes(0x10), memory_size=0x40, perm=PERM_RW)])
        math = lib("math", deps=[Dependency("base")],
                   imports=[Import(0, "add", FUNCTION, 0x80, 0, "fn(u32,u32)->u32"),
                            Import(0, "counter", OBJECT, 0x88, 4)])
        exe = app(deps=[Dependency("math"), Dependency("base")],
                  imports=[imp(0, slot=0x100), imp(0, slot=0x108),
                           Import(1, "counter", OBJECT, 0x110, 4)])
        r = self.ok(exe, [math, base])
        self.assertEqual(r.count, 3)
        app_origin, math_origin, base_origin = r.origins[:3]
        # Dependencies are placed first (post-order): base, then math.
        self.assertEqual(base_origin, 0x100180)
        self.assertEqual(math_origin, 0x100240)
        self.assertEqual((r.word(0x100100), r.word(0x100104)), (math_origin + 0x20, math_origin))
        self.assertEqual((r.word(0x100108), r.word(0x10010C)), (math_origin + 0x20, math_origin))
        self.assertEqual((r.word(0x100110), r.word(0x100114)), (base_origin + 0x90, base_origin))
        self.assertEqual((r.word(math_origin + 0x80), r.word(math_origin + 0x84)), (base_origin, base_origin))
        self.assertEqual(r.word(math_origin + 0x88), base_origin + 0x90)
        self.assertEqual(r.word(base_origin + 0x90), 0)  # exported BSS object, zeroed
        # The APP RW block, base and math are adjacent: equal permissions merge
        # only where the blocks touch.
        self.assertEqual(r.regions, [
            (0x1FF000, 0x200000, PERM_RW), (0x100000, 0x100100, PERM_RX),
            (0x100100, 0x100180, PERM_RW), (0x100180, 0x100200, PERM_RX),
            (0x100200, 0x100240, PERM_RW), (0x100240, 0x1002C0, PERM_RX),
            (0x1002C0, 0x100300, PERM_RW)])

    def test_same_wal_at_two_bases(self):
        console = lib("cpt32.console", align=0x1000)
        exe = app(deps=[Dependency("cpt32.console")], imports=[imp()])
        first = self.ok(exe, [console])
        moved = app(base=0x140000, deps=[Dependency("cpt32.console")], imports=[imp()])
        second = self.ok(moved, [console], window=(0x140000, 0x200000))
        self.assertEqual(first.origins[1], 0x101000)
        self.assertEqual(second.origins[1], 0x141000)
        self.assertEqual(first.word(0x100100), 0x101020)
        self.assertEqual(second.word(0x140100), 0x141020)

    def test_provider_selection(self):
        exe = app(deps=[Dependency("cpt32.console", 1, 1)], imports=[imp()])
        old, new, other_major = lib("cpt32.console", minor=2), lib("cpt32.console", minor=5, align=0x10000), \
            lib("cpt32.console", major=2, minor=9)
        r = self.ok(exe, [old, other_major, new])
        self.assertEqual(r.origins[1], 0x110000)  # the highest minor of major 1
        tie = lib("cpt32.console", minor=2, align=0x10000)
        self.assertEqual(self.ok(exe, [old, tie]).origins[1], 0x100180)  # first listed
        broken = lib("cpt32.console", minor=7)
        broken.put32(broken.layout["blocks"] + 16, 7)  # WX: not a provider
        self.assertEqual(self.ok(exe, [broken, old]).origins[1], 0x100180)

    def test_minor_requirements_combine_by_maximum(self):
        b = lib("b", deps=[Dependency("a", 1, 5)], imports=[imp(slot=0x80)])
        exe = app(deps=[Dependency("a", 1, 3), Dependency("b")], imports=[imp(0), imp(1, slot=0x108)])
        self.rejects(DEPENDENCY_MISSING, exe, [lib("a", minor=4), b])
        self.ok(exe, [lib("a", minor=5), b])

    def test_exactly_eight_regions(self):
        def wal(name, deps=()):
            return lib(name, deps=[Dependency(d) for d in deps],
                       imports=[imp(i, slot=0x80 + 8 * i) for i in range(len(deps))],
                       blocks=[Block(0, bytes(0x80), perm=PERM_RX, alignment=0x100),
                               Block(0x80, bytes(0x40), perm=PERM_RW),
                               Block(0x100, bytes(0x20), perm=PERM_R)])
        two = [wal("p"), wal("q")]
        exe = app(deps=[Dependency("p"), Dependency("q")], imports=[imp(0), imp(1, slot=0x108)])
        # stack + APP RX/RW + 2 x (RX, RW, hole, R) = 1 + 2 + 6 = 9 regions.
        self.rejects(REGION_LIMIT, exe, two)
        # stack + APP RX/RW + p (RX, RW, hole, R) + two single-block
        # libraries that are not adjacent = 1 + 2 + 3 + 1 + 1 = 8.
        exe = app(deps=[Dependency("p"), Dependency("r"), Dependency("s")],
                  imports=[imp(0), imp(1, slot=0x108), imp(2, slot=0x110)])
        r_lib = lib("r", blocks=[Block(0, bytes(0x80), perm=PERM_RX, alignment=0x100)])
        s_lib = lib("s", blocks=[Block(0, bytes(0x80), perm=PERM_RX, alignment=0x100)])
        r = self.ok(exe, [wal("p"), r_lib, s_lib])
        self.assertEqual(len(r.regions), 8)
        self.assertEqual(r.origins[1:4], [0x100200, 0x100400, 0x100500])
        # Packed adjacently, equal-permission blocks of different libraries
        # share a region.
        r_lib = lib("r", blocks=[Block(0, bytes(0x80), perm=PERM_RX)])
        s_lib = lib("s", blocks=[Block(0, bytes(0x80), perm=PERM_RX)])
        r = self.ok(exe, [wal("p"), r_lib, s_lib])
        # r fills the gap below p's aligned base and touches p's RX block.
        self.assertEqual(r.origins[1:4], [0x100200, 0x100180, 0x100320])
        self.assertIn((0x100180, 0x100280, PERM_RX), r.regions)

    def test_hole_or_permission_prevents_merging(self):
        blocks = [Block(0x100000, bytes(0x40), perm=PERM_RX), Block(0x100040, bytes(0x40), perm=PERM_RX),
                  Block(0x100100, bytes(0x40), perm=PERM_RX), Block(0x100140, bytes(0x40), perm=PERM_R)]
        r = self.ok(app(blocks=blocks))
        self.assertEqual(r.regions[1:], [(0x100000, 0x100080, PERM_RX), (0x100100, 0x100140, PERM_RX),
                                         (0x100140, 0x100180, PERM_R)])

    def test_protected_v2(self):
        image = wax.pack(0x100000, 4, [(0x100000, bytes(range(16)), 5), (0x100010, b"\x09" * 8, 3)],
                         protection=(0x200000, 0x1000))
        r = self.ok(image)
        self.assertEqual((r.entry, r.stack_top), (0x100004, 0x200000))
        self.assertEqual(r.regions, [(0x1FF000, 0x200000, PERM_RW), (0x100000, 0x100010, PERM_RX),
                                     (0x100010, 0x100018, PERM_RW)])
        self.assertEqual(r.ram[0xFF000:0x100000], bytes(0x1000))
        self.rejects(FORMAT, wax.pack(0x100000, 0, [(0x100000, bytes(16))]))  # not PROTECTED
        base = ctypes.c_uint32()
        self.assertEqual(self.lib.host_base_addr(image, len(image), ctypes.byref(base)), OK)
        self.assertEqual(base.value, 0x100000)

    # --- rejection: structure ---

    def test_header_identity(self):
        for offset, value, error in ((4, 4, UNSUPPORTED), (6, 3, UNSUPPORTED), (8, 2, UNSUPPORTED),
                                     (10, 6, UNSUPPORTED), (10, 0, FORMAT)):
            image = app()
            image.data[offset:offset + 2] = value.to_bytes(2, "big")
            self.rejects(error, image)
        unprotected = app(protected=False)
        self.rejects(FORMAT, unprotected)
        self.rejects(UNSUPPORTED, app(call_abi=2))
        self.rejects(FORMAT, lib("x"))  # a WAL is not launchable
        reserved = app()
        reserved.put32(140, 1)
        self.rejects(FORMAT, reserved)
        truncated = app()
        self.rejects(FORMAT, truncated.data[:-4])

    def test_metadata_and_payload_overlap(self):
        image = app()
        image.put32(image.layout["blocks"] + 4, image.layout["strings"])
        self.rejects(FORMAT, image)
        image = app()
        image.put32(image.layout["blocks"] + 24 + 4, image.layout["payloads"][0] + 4)
        self.rejects(FORMAT, image)
        image = app(debug=b"dbg!")
        image.put32(124, 0x40)  # debug overruns the file
        self.rejects(FORMAT, image)
        padding = app()
        padding.data += b"\0\0\0\1"  # nonzero padding at the end of the file
        padding.put32(16, len(padding.data))
        self.rejects(FORMAT, padding)
        padding.data[-1] = 0
        self.ok(padding)

    def test_overflow(self):
        image = app()
        image.put32(80, 0x0AAAAAAB)  # import_count * 24 wraps u32
        image.put32(76, 0x100)
        self.rejects(FORMAT, image)
        image = app(blocks=[Block(0x100000, bytes(0x40)), Block(0xFFFFFF00, b"", memory_size=0x200, perm=PERM_RW)])
        self.rejects(FORMAT, image)
        image = app()
        image.put32(44, 1000)
        self.rejects(RESOURCE_LIMIT, image)

    def test_table_strides(self):
        for offset in (48, 60, 72, 84):
            image = app()
            image.put32(offset, image.get32(offset) + 4)
            self.rejects(FORMAT, image)

    def test_strings(self):
        image = app()
        image.put32(96, image.get32(96) + 1)  # interior of "app"
        self.rejects(FORMAT, image)
        image = app()
        image.put32(96, 0)
        self.rejects(FORMAT, image)
        for bad in ("App", "a..b", "a-b", "1a", "a." , "x" * 128):
            self.rejects(FORMAT, build("wax", image_id=bad, blocks=[Block(0x100000, bytes(8))],
                                       stack_top=0x200000, stack_size=0x100))
        self.ok(build("wax", image_id="x" * 127, blocks=[Block(0x100000, bytes(8))],
                      stack_top=0x200000, stack_size=0x100))
        image = app()
        image.data[image.layout["strings"] + 1] = 0xE1  # non-ASCII
        self.rejects(FORMAT, image)

    def test_exports(self):
        exe = app(deps=[Dependency("l")], imports=[imp()])
        dup = [Export("write", FUNCTION, 0x20, 0x10, SIG), Export("write", OBJECT, 0x80, 4)]
        unsorted = [Export("write", FUNCTION, 0x20, 0x10, SIG), Export("read", FUNCTION, 0x20, 0x10, SIG)]
        for exports in (dup, unsorted,
                        [Export("write", FUNCTION, 0x20, 0x10, "fn(u32) -> u32")],
                        [Export("write", FUNCTION, 0x20, 0x10, "fn(u8)->u32")],
                        [Export("write", FUNCTION, 0x20, 0x10, "fn(void)->u32")],
                        [Export("write", FUNCTION, 0x20, 0x10, "fn(u32,)->u32")],
                        [Export("write", FUNCTION, 0x22, 0x10, SIG)],          # unaligned
                        [Export("write", FUNCTION, 0x20, 0x0E, SIG)],          # size % 4
                        [Export("write", FUNCTION, 0x78, 0x10, SIG)],          # crosses into RW
                        [Export("write", FUNCTION, 0x80, 0x10, SIG)],          # in RW
                        [Export("write", OBJECT, 0x20, 4)],                    # object in RX
                        [Export("write", OBJECT, 0xB0, 0x20)],                 # past the block
                        [Export("9write", FUNCTION, 0x20, 0x10, SIG)]):
            with self.subTest(exports=exports):
                self.rejects(DEPENDENCY_MISSING, exe, [lib("l", exports=exports)])
        sigs = ["fn()->void", "fn(fnptr,u64,i64,ptr,i32)->fnptr", "fn(u32)->u64"]
        exports = [Export("e%d" % i, FUNCTION, 0, 4, s) for i, s in enumerate(sigs)]
        self.ok(app(deps=[Dependency("l")], imports=[imp(name="e0", signature=sigs[0])]),
                [lib("l", exports=exports)])

    def test_imports_and_dependencies(self):
        cases = {
            "slot in R": dict(blocks=[Block(0x100000, bytes(0x100)), Block(0x100100, bytes(0x80), perm=PERM_R)]),
            "slot crosses block": dict(imports=[imp(slot=0x17C)]),
            "slot unaligned": dict(imports=[imp(slot=0x102)]),
            "slot not zero": dict(data=b"\x00\x00\x00\x01" + bytes(0x7C)),
            "slots overlap": dict(imports=[imp(slot=0x100), imp(slot=0x104)]),
            "slots unsorted": dict(imports=[imp(slot=0x108), imp(slot=0x100)]),
            "unused dependency": dict(deps=[Dependency("l"), Dependency("m")]),
            "duplicate dependency": dict(deps=[Dependency("l"), Dependency("l")],
                                         imports=[imp(0), imp(1, slot=0x108)]),
            "self dependency": dict(deps=[Dependency("app")]),
            "bad index": dict(imports=[imp(dep=1)]),
            "zero major": dict(deps=[Dependency("l", 0)]),
            "function with size": dict(imports=[imp(size=4)]),
            "object with signature": dict(imports=[Import(0, "write", OBJECT, 0x100, 4, SIG)]),
        }
        for name, kw in cases.items():
            with self.subTest(name):
                kw.setdefault("deps", [Dependency("l")])
                kw.setdefault("imports", [imp()])
                self.rejects(FORMAT, app(**kw), [lib("l")])
        reserved = app(deps=[Dependency("l")], imports=[imp()])
        reserved.put32(reserved.layout["deps"] + 12, 1)
        self.rejects(FORMAT, reserved, [lib("l")])
        bss_slot = app(data=b"", deps=[Dependency("l")], imports=[imp()],
                       blocks=[Block(0x100000, bytes(0x100)), Block(0x100100, b"", memory_size=0x80, perm=PERM_RW)])
        self.ok(bss_slot, [lib("l")])

    def test_blocks_entry_and_stack(self):
        cases = {
            "WX": [Block(0x100000, bytes(0x40), perm=7)],
            "X only": [Block(0x100000, bytes(0x40), perm=4)],
            "alignment not power of two": [Block(0x100000, bytes(0x40), alignment=12)],
            "alignment below 4": [Block(0x100000, bytes(0x40), alignment=2)],
            "address not aligned": [Block(0x100010, bytes(0x40), alignment=0x20)],
            "overlap": [Block(0x100000, bytes(0x40)), Block(0x100020, bytes(0x40), perm=PERM_RW)],
            "unsorted": [Block(0x100100, bytes(0x40)), Block(0x100000, bytes(0x40), perm=PERM_RW)],
            "memory below file": [Block(0x100000, bytes(0x40), memory_size=0x20)],
        }
        for name, blocks in cases.items():
            with self.subTest(name):
                self.rejects(FORMAT, app(blocks=blocks))
        self.rejects(FORMAT, app(entry_offset=2))
        self.rejects(FORMAT, app(entry_offset=0x100))  # entry in RW
        bss_entry = app(blocks=[Block(0x100000, bytes(0x40), memory_size=0x80)], entry_offset=0x40)
        self.rejects(FORMAT, bss_entry)
        self.rejects(FORMAT, app(stack=(0x100100, 0x100200)))     # overlaps the image
        self.rejects(FORMAT, app(stack=(0x1FF004, 0x200000)))     # size not a multiple of 8
        self.rejects(FORMAT, app(stack=(0x200000, 0x200000)))     # empty
        wal_base = lib("l", blocks=[Block(0x10, bytes(0x80))])
        self.rejects(DEPENDENCY_MISSING, app(deps=[Dependency("l")], imports=[imp()]), [wal_base])

    def test_window(self):
        r = self.plan(app(), window=(0x100000, 0x1FF800))
        self.assertEqual(r.error, FORMAT)  # stack outside the window (slot)
        r = self.plan(app(base=0xF0000), window=WINDOW)
        self.assertEqual(r.error, FORMAT)
        big = lib("l", blocks=[Block(0, bytes(0x80)), Block(0x80, b"", memory_size=0x100000, perm=PERM_RW)])
        r = self.plan(app(deps=[Dependency("l")], imports=[imp()]), [big])
        self.assertEqual((r.error, r.diag_image), (FORMAT, 1))

    # --- rejection: graph and symbols ---

    def test_graph(self):
        exe = app(deps=[Dependency("l")], imports=[imp()])
        r = self.plan(exe)
        self.assertEqual((r.error, r.diag_image, r.diag_index, r.diag_name), (DEPENDENCY_MISSING, 0, 0, "l"))
        self.rejects(DEPENDENCY_MISSING, exe, [lib("l", major=2)])
        self.rejects(DEPENDENCY_MISSING, app(deps=[Dependency("l", 1, 3)], imports=[imp()]), [lib("l", minor=2)])
        a1 = lib("a", deps=[Dependency("b")], imports=[imp(slot=0x80)])
        b_cycle = lib("b", deps=[Dependency("a")], imports=[imp(slot=0x80)])
        r = self.plan(app(deps=[Dependency("a")], imports=[imp()]), [a1, b_cycle])
        self.assertEqual((r.error, r.diag_name), (DEPENDENCY_CYCLE, "a"))
        b_major = lib("b", deps=[Dependency("a", 2)], imports=[imp(slot=0x80)])
        conflict = app(deps=[Dependency("a"), Dependency("b")], imports=[imp(0), imp(1, slot=0x108)])
        self.rejects(DEPENDENCY_CONFLICT, conflict, [lib("a"), lib("a", major=2), b_major])
        self.rejects(ABI_MISMATCH, exe, [lib("l", os_id="other")])
        self.rejects(ABI_MISMATCH, exe, [lib("l", os_major=2)])
        self.rejects(ABI_MISMATCH, exe, [lib("l", os_min_minor=1)])
        self.rejects(ABI_MISMATCH, app(os_min_minor=1))
        self.rejects(ABI_MISMATCH, app(os_id="other"))
        # A WAX is never a provider, and a v2 file has no symbols.
        self.rejects(DEPENDENCY_MISSING, exe, [build("wax", image_id="l", blocks=[Block(0x100000, bytes(8))],
                                                     stack_top=0x200000, stack_size=0x100)])

    def test_graph_limit(self):
        names = ["l%d" % i for i in range(8)]
        libs = [lib(n, deps=[Dependency(names[i + 1])] if i + 1 < len(names) else (),
                    imports=[imp(slot=0x80)] if i + 1 < len(names) else ())
                for i, n in enumerate(names)]
        self.rejects(RESOURCE_LIMIT, app(deps=[Dependency("l0")], imports=[imp()]), libs)

    def test_symbols(self):
        exe = lambda *i: app(deps=[Dependency("l")], imports=list(i))  # noqa: E731
        r = self.plan(exe(imp(name="read")), [lib("l")])
        self.assertEqual((r.error, r.diag_image, r.diag_index, r.diag_name), (SYMBOL_MISSING, 0, 0, "read"))
        self.rejects(SYMBOL_MISMATCH, exe(imp(signature="fn(ptr,u32)->i32")), [lib("l")])
        self.rejects(SYMBOL_MISMATCH, exe(imp(kind=OBJECT, size=0x10)), [lib("l")])
        objects = lib("l", exports=[Export("table", OBJECT, 0x80, 8)])
        self.rejects(SYMBOL_MISMATCH, exe(imp(name="table", kind=OBJECT, size=4)), [objects])
        self.rejects(SYMBOL_MISMATCH, exe(imp(name="table")), [objects])
        self.ok(exe(imp(name="table", kind=OBJECT, size=8)), [objects])
        # Only direct dependencies are visible.
        inner = lib("inner", exports=[Export("deep", FUNCTION, 0, 4, "fn()->void")])
        outer = lib("outer", deps=[Dependency("inner")],
                    imports=[Import(0, "deep", FUNCTION, 0x80, 0, "fn()->void")])
        self.rejects(SYMBOL_MISSING, app(deps=[Dependency("outer")],
                                         imports=[Import(0, "deep", FUNCTION, 0x100, 0, "fn()->void")]),
                     [outer, inner])


if __name__ == "__main__":
    unittest.main()
