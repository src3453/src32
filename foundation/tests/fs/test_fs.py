"""Host tests of the fs service core (foundation/fs).

The FS2 driver, the VFS and the protocol of doc/spec_Foundation_API.md §5-§7
are built with the host C compiler and driven through host.c, which stands
in for the IPC main loop. Images made or changed here are checked with the
host tool tools/fs2/fs2.py when it is available, so that the guest driver
and the host tool agree on the format (doc/spec_FS2.md §12 step 2).

Run: python -m unittest discover -s foundation/tests/fs -v
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
FOUNDATION = ROOT / "foundation"
sys.path.insert(0, str(ROOT / "tools/fs2"))
sys.path.insert(0, str(ROOT / "tools/dsfs"))
import dsfs  # noqa: E402
try:
    import fs2 as fs2tool  # noqa: E402
except ImportError:  # the host tool lands with the disk boot work
    fs2tool = None

# Mounts (foundation/fs/vfs.h).
ROOT_MNT, DEV, DD, OS, TMP, DD0 = 0, 1, 2, 3, 4, 5

# Errors (spec_Foundation.md §4, spec_Foundation_API.md §3).
E = dict(BAD_ARG=-2, BAD_HANDLE=-3, ACCESS=-4, NO_MEMORY=-5, LIMIT=-6, NOT_FOUND=-10, EXISTS=-11,
         FORMAT=-12, UNSUPPORTED=-13, IO=-16, READ_ONLY=-18, BUSY=-19, NOT_DIR=-64, IS_DIR=-65,
         NOT_EMPTY=-66, BAD_PATH=-67, NAME_TOO_LONG=-68, NO_SPACE=-69, CROSS_DEVICE=-70, NEEDS_FSCK=-72)

ST_READ, ST_WRITE, ST_SEEK, ST_STAT, ST_TRUNCATE, ST_SYNC = range(1, 7)
(FS_OPEN, FS_STAT, FS_DIR_OPEN, FS_MKDIR, FS_UNLINK, FS_RMDIR, FS_RENAME, FS_CHMOD, FS_CHOWN, FS_SYNC,
 FS_CWD_SET, FS_CWD_GET, FS_SESSION_NEW, FS_VOLUME_INFO) = range(0x100, 0x10E)
DIR_NEXT, DIR_REWIND = 0x200, 0x201
READ, WRITE, CREATE, EXCL, TRUNC, APPEND, EXEC = (1 << i for i in range(7))
DEFAULT = 0xFFFFFFFF
SESSION = 0x80000001


def s32(v):
    return v - (1 << 32) if v & 0x80000000 else v


def be(data, off, n=4):
    return int.from_bytes(data[off:off + n], "big")


class Reply:
    def __init__(self, out, reply):
        self.status = s32(out[0])
        self.a = list(out[1:5])
        self.new_badge, self.fwd = out[5], out[6]
        self.data = bytes(reply[:out[7]])


def stat_of(data):
    return dict(type=be(data, 0), mode=be(data, 4), uid=be(data, 8), flags=be(data, 12),
                size=be(data, 16, 8), created=(s32(be(data, 24)) << 32 | be(data, 28), be(data, 32)),
                modified=(be(data, 36, 8), be(data, 44)), changed=(be(data, 48, 8), be(data, 56)))


def entries_of(data):
    out, off = [], 0
    while off < len(data):
        rec = be(data, off)
        nb, nc = be(data, off + 68, 2), be(data, off + 70, 2)
        out.append((data[off + 72:off + 72 + nb].decode(), nc, stat_of(data[off + 4:off + 68])))
        off += rec
    return out


class Host:
    """The fs core with images in Python buffers."""

    def __init__(self, lib):
        self.lib = lib
        self.images = {}
        lib.host_init()
        lib.host_set_time(1_700_000_000, 0)

    def format(self, sectors=2880, inodes=256, boot_blocks=0, total=None, root_mode=0x3D):
        image = (ctypes.c_uint8 * (sectors * 512))()
        st = s32(self.lib.host_format(image, sectors, total or sectors, boot_blocks, inodes, root_mode) & 0xFFFFFFFF)
        assert st == 0, st
        return image

    def attach(self, mnt, image, writable=True, read_only=False, fs_type=1, slot=0, alias=-1):
        self.images[mnt] = image
        return s32(self.lib.host_attach(mnt, image, len(image) // 512, int(writable), int(read_only), fs_type,
                                        slot, alias) & 0xFFFFFFFF)

    def boot_disk(self, image=None, **kw):
        image = image if image is not None else self.format(**kw)
        assert self.attach(ROOT_MNT, image, alias=DD0) == 0
        tmp = self.format(sectors=256, inodes=32, root_mode=0x3F)
        assert self.attach(TMP, tmp, fs_type=3, slot=0xFFFFFFFF) == 0
        return image

    def call(self, badge, op, a0=0, a1=0, a2=0, a3=0, data=b"", cap=4096, flags=0, total=None):
        out = (ctypes.c_uint32 * 8)()
        reply = (ctypes.c_uint8 * 4096)()
        if isinstance(data, str):
            data = data.encode()
        buf = (ctypes.c_uint8 * max(len(data), 1)).from_buffer_copy(data or b"\0")
        self.lib.host_request(badge, op, flags, a0, a1, a2, a3, buf, len(data),
                              len(data) if total is None else total, cap, out, reply)
        return Reply(out, reply)

    # Convenience wrappers that assert success unless told otherwise.
    def ok(self, *args, **kw):
        r = self.call(*args, **kw)
        assert r.status >= 0, (args, r.status)
        return r

    def open(self, path, flags=READ, mode=0, session=SESSION):
        r = self.ok(session, FS_OPEN, flags, mode, data=path)
        return r.new_badge

    def write(self, h, data):
        for i in range(0, len(data), 4096):
            r = self.ok(h, ST_WRITE, data=data[i:i + 4096])
            assert r.a[0] == len(data[i:i + 4096])

    def read_all(self, h, chunk=4096):
        out = b""
        while True:
            r = self.ok(h, ST_READ, chunk)
            if not r.data:
                return out
            out += r.data

    def put(self, path, data, session=SESSION):
        h = self.open(path, WRITE | CREATE | TRUNC, DEFAULT, session)
        self.write(h, data)
        self.lib.host_gone(h)

    def get(self, path, session=SESSION):
        h = self.open(path, READ, session=session)
        data = self.read_all(h)
        self.lib.host_gone(h)
        return data

    def list(self, path, cap=4096, session=SESSION):
        d = self.ok(session, FS_DIR_OPEN, data=path).new_badge
        names = []
        while True:
            r = self.ok(d, DIR_NEXT, cap=cap)
            if r.a[0] == 0:
                break
            names += entries_of(r.data)
        self.lib.host_gone(d)
        return names

    def remount(self, mnt=ROOT_MNT, alias=DD0):
        assert s32(self.lib.host_unmount(mnt) & 0xFFFFFFFF) == 0
        return self.attach(mnt, self.images[mnt], alias=alias)


class FsTestBase(unittest.TestCase):
    mount_cflags = []

    @classmethod
    def setUpClass(cls):
        cc = os.environ.get("CC") or shutil.which("cc") or shutil.which("gcc") or shutil.which("clang")
        if not cc:
            raise unittest.SkipTest("no host C compiler (set CC)")
        cls.temp = tempfile.TemporaryDirectory(prefix="fs-host-")
        library = Path(cls.temp.name) / ("fshost" + (".dll" if os.name == "nt" else ".so"))
        sources = [HERE / "host.c", *sorted((FOUNDATION / "fs").glob("*.c")), FOUNDATION / "lib/fnd/name.c",
                   FOUNDATION / "lib/fnd/name_api.c"]
        sources = [s for s in sources if s.name != "main.c"]
        extra = os.environ.get("FS_TEST_CFLAGS", "").split()
        pic = [] if os.name == "nt" else ["-fPIC"]
        subprocess.run([cc, "-std=c11", "-O1", "-Wall", "-Wextra", "-Werror", "-shared", *pic, *extra,
                        *cls.mount_cflags,
                        "-I", str(FOUNDATION / "include"), "-I", str(FOUNDATION / "lib/fnd"),
                        "-I", str(FOUNDATION / "fs"), *map(str, sources), "-o", str(library)], check=True)
        cls.lib = ctypes.CDLL(str(library))
        cls.lib.host_set_time.argtypes = [ctypes.c_int64, ctypes.c_uint32]
        cls.lib.host_attach.argtypes = [ctypes.c_uint32, ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32,
                                        ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_int32]
        cls.lib.host_attach_dsfs.argtypes = [ctypes.c_uint32, ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32]
        cls.lib.host_format.argtypes = [ctypes.c_void_p] + [ctypes.c_uint32] * 5

    @classmethod
    def tearDownClass(cls):
        if os.name == "nt":
            # Windows locks a loaded DLL, even when its CDLL is collected.
            import _ctypes
            _ctypes.FreeLibrary(cls.lib._handle)
        del cls.lib
        cls.temp.cleanup()

    def setUp(self):
        self.fs = Host(self.lib)

    def status(self, r):
        return r.status

    def assertError(self, name, reply):
        self.assertEqual(reply.status, E[name], f"expected {name}, got {reply.status}")

    def image_bytes(self, mnt=ROOT_MNT):
        return bytes(self.fs.images[mnt])


class FileTests(FsTestBase):
    def test_boundary_sizes_round_trip_and_remount(self):
        fs = self.fs
        fs.boot_disk()
        sizes = [0, 1, 507, 508, 509, 1016, 1017, 5000, 9000]
        for n in sizes:
            fs.put(f"/f{n}", bytes((i * 7 + n) & 0xFF for i in range(n)))
        self.assertEqual(fs.remount(), 0)
        for n in sizes:
            self.assertEqual(fs.get(f"/f{n}"), bytes((i * 7 + n) & 0xFF for i in range(n)))
            st = stat_of(fs.ok(SESSION, FS_STAT, data=f"/f{n}").data)
            self.assertEqual((st["type"], st["size"], st["mode"]), (1, n, 0x34))

    def test_seek_past_end_fills_holes_with_zero(self):
        fs = self.fs
        fs.boot_disk()
        h = fs.open("/sparse", WRITE | READ | CREATE, DEFAULT)
        fs.ok(h, ST_SEEK, 0, 0, 2000)
        fs.write(h, b"xyz")
        r = fs.ok(h, ST_SEEK, 0, 0, 0)
        self.assertEqual(r.a[:2], [0, 0])
        self.assertEqual(fs.read_all(h), bytes(2000) + b"xyz")
        fs.ok(h, ST_SEEK, 2, 0xFFFFFFFF, 0xFFFFFFFE)  # END - 2
        self.assertEqual(fs.ok(h, ST_READ, 10).data, b"yz")
        self.assertError("BAD_ARG", fs.call(h, ST_SEEK, 0, 0xFFFFFFFF, 0xFFFFFFFF))
        fs.lib.host_gone(h)
        self.assertEqual(fs.remount(), 0)

    def test_truncate_and_append(self):
        fs = self.fs
        fs.boot_disk()
        fs.put("/t", bytes(range(256)) * 10)
        h = fs.open("/t", READ | WRITE)
        fs.ok(h, ST_TRUNCATE, 0, 0, 600)
        self.assertEqual(stat_of(fs.ok(h, ST_STAT).data)["size"], 600)
        fs.ok(h, ST_TRUNCATE, 0, 0, 1100)
        self.assertEqual(fs.read_all(h), (bytes(range(256)) * 10)[:600] + bytes(500))
        fs.ok(h, ST_TRUNCATE, 0, 0, 0)
        fs.lib.host_gone(h)
        a = fs.open("/t", WRITE | APPEND)
        fs.write(a, b"one")
        fs.ok(a, ST_SEEK, 0, 0, 0)
        fs.write(a, b"two")
        fs.lib.host_gone(a)
        self.assertEqual(fs.get("/t"), b"onetwo")
        self.assertEqual(fs.remount(), 0)

    def test_open_flags(self):
        fs = self.fs
        fs.boot_disk()
        fs.put("/a", b"data")
        self.assertError("EXISTS", fs.call(SESSION, FS_OPEN, WRITE | CREATE | EXCL, DEFAULT, data="/a"))
        self.assertError("NOT_FOUND", fs.call(SESSION, FS_OPEN, READ, data="/b"))
        for bad in (0, CREATE, WRITE | EXEC, READ | TRUNC, READ | APPEND, READ | EXCL, 0x80):
            self.assertError("BAD_ARG", fs.call(SESSION, FS_OPEN, bad, data="/a"))
        self.assertError("BAD_ARG", fs.call(SESSION, FS_OPEN, WRITE | CREATE, 0x40, data="/c"))
        r = fs.ok(SESSION, FS_OPEN, EXEC, data="/a")
        self.assertEqual(r.a[0], 1)
        self.assertError("ACCESS", fs.call(r.new_badge, ST_WRITE, data=b"x"))
        self.assertEqual(fs.ok(r.new_badge, ST_READ, 100).data, b"data")
        ro = fs.open("/a")
        self.assertError("ACCESS", fs.call(ro, ST_WRITE, data=b"x"))
        self.assertError("ACCESS", fs.call(ro, ST_TRUNCATE))
        self.assertError("BAD_ARG", fs.call(ro, ST_READ, 0))
        self.assertError("BAD_ARG", fs.call(ro, ST_READ, 4097))
        self.assertError("BAD_ARG", fs.call(ro, ST_READ, 1, 1))
        self.assertError("UNSUPPORTED", fs.call(ro, FS_STAT, data="/a"))
        self.assertError("UNSUPPORTED", fs.call(ro, 0x1000))
        self.assertError("IS_DIR", fs.call(SESSION, FS_OPEN, READ, data="/"))
        self.assertError("IS_DIR", fs.call(SESSION, FS_OPEN, READ, data="/tmp"))
        self.assertError("NOT_DIR", fs.call(SESSION, FS_OPEN, READ, data="/a/"))
        self.assertError("BAD_ARG", fs.call(SESSION, FS_STAT, data="/a", flags=1))

    def test_read_respects_reply_capacity(self):
        fs = self.fs
        fs.boot_disk()
        fs.put("/r", bytes(3000))
        h = fs.open("/r")
        self.assertEqual(len(fs.ok(h, ST_READ, 4096, cap=100).data), 100)
        self.assertEqual(len(fs.ok(h, ST_READ, 4096).data), 2900)
        self.assertEqual(fs.ok(h, ST_READ, 4096).data, b"")

    def test_dup_shares_position_and_gone_releases(self):
        fs = self.fs
        fs.boot_disk()
        fs.put("/p", b"abcdef")
        h = fs.open("/p")
        self.assertEqual(fs.ok(h, ST_READ, 2).data, b"ab")
        self.assertEqual(fs.ok(h, ST_READ, 2).data, b"cd")  # same CHANNEL, same object
        self.assertError("BUSY", fs.call(SESSION, FS_UNLINK, data="/p"))
        fs.lib.host_gone(h)
        self.assertError("BAD_HANDLE", fs.call(h, ST_READ, 1))
        fs.ok(SESSION, FS_UNLINK, data="/p")
        self.assertError("NOT_FOUND", fs.call(SESSION, FS_STAT, data="/p"))


class PathTests(FsTestBase):
    def test_paths_and_cwd(self):
        fs = self.fs
        fs.boot_disk()
        fs.ok(SESSION, FS_MKDIR, 0, DEFAULT, data="/usr")
        fs.ok(SESSION, FS_MKDIR, 0, 0x3F, data="/usr//Bin/")
        fs.put("/usr/bin/tool", b"T")
        fs.ok(SESSION, FS_CWD_SET, data="usr/BIN")
        self.assertEqual(fs.ok(SESSION, FS_CWD_GET).data, b"/usr/Bin")
        self.assertEqual(fs.get("tool"), b"T")
        self.assertEqual(fs.get("./../bin/./TOOL"), b"T")
        self.assertEqual(fs.get("/../../usr/bin/tool"), b"T")
        self.assertError("NOT_FOUND", fs.call(SESSION, FS_STAT, data="/missing/../usr"))
        self.assertError("NOT_DIR", fs.call(SESSION, FS_STAT, data="tool/.."))
        self.assertError("NOT_DIR", fs.call(SESSION, FS_CWD_SET, data="tool"))
        self.assertError("LIMIT", fs.call(SESSION, FS_CWD_GET, cap=3))
        st = stat_of(fs.ok(SESSION, FS_STAT, data="/usr/bin").data)
        self.assertEqual((st["type"], st["mode"]), (2, 0x3F))
        # A new session copies the cwd; changing it leaves the original.
        child = fs.ok(SESSION, FS_SESSION_NEW).new_badge
        self.assertEqual(fs.ok(child, FS_CWD_GET).data, b"/usr/Bin")
        fs.ok(child, FS_CWD_SET, data="/")
        self.assertEqual(fs.ok(SESSION, FS_CWD_GET).data, b"/usr/Bin")
        self.assertEqual(fs.get("usr/bin/tool", session=child), b"T")
        # The cwd keeps its directory busy (§11.2).
        self.assertError("BUSY", fs.call(child, FS_RENAME, 4, data="/usr/abc"))
        fs.ok(SESSION, FS_CWD_SET, data="/")
        fs.ok(child, FS_RENAME, 4, data="/usr/abc")
        self.assertEqual(fs.get("/abc/bin/tool"), b"T")

    def test_path_errors(self):
        fs = self.fs
        fs.boot_disk()
        self.assertError("BAD_PATH", fs.call(SESSION, FS_STAT, data=""))
        self.assertError("BAD_PATH", fs.call(SESSION, FS_STAT, data="/a\0b"))
        self.assertError("BAD_PATH", fs.call(SESSION, FS_MKDIR, 0, DEFAULT, data="/bad\x01"))
        self.assertError("BAD_PATH", fs.call(SESSION, FS_MKDIR, 0, DEFAULT, data=b"/\xc0\xaf"))
        self.assertError("BAD_PATH", fs.call(SESSION, FS_MKDIR, 0, DEFAULT, data=b"/\xed\xa0\x80"))
        self.assertError("NAME_TOO_LONG", fs.call(SESSION, FS_MKDIR, 0, DEFAULT, data="/" + "a" * 257))
        self.assertError("NAME_TOO_LONG", fs.call(SESSION, FS_STAT, data="/a", total=5000))
        fs.ok(SESSION, FS_MKDIR, 0, DEFAULT, data="/" + "a" * 256)
        fs.ok(SESSION, FS_MKDIR, 0, DEFAULT, data="/" + "日" * 256)
        fs.ok(SESSION, FS_MKDIR, 0, DEFAULT, data="/" + "\U0001F600" * 256)
        self.assertError("NAME_TOO_LONG", fs.call(SESSION, FS_MKDIR, 0, DEFAULT, data="/" + "日" * 257))
        # §6.2: the absolute form of a relative path is limited as well.
        fs.ok(SESSION, FS_CWD_SET, data="/" + "a" * 256)
        self.assertError("NAME_TOO_LONG", fs.call(SESSION, FS_STAT, data="b" * 3840))
        self.assertError("NAME_TOO_LONG", fs.call(SESSION, FS_RENAME, 1, data="x" + "y" * 4095, total=4097))

    def test_case_folding(self):
        fs = self.fs
        fs.boot_disk()
        fs.put("/Straße", b"1")
        self.assertEqual(fs.get("/STRASSE"), b"1")
        self.assertError("EXISTS", fs.call(SESSION, FS_OPEN, WRITE | CREATE | EXCL, DEFAULT, data="/strasse"))
        fs.put("/é", b"composed")
        fs.put("/é", b"combining")
        self.assertEqual(fs.get("/É"), b"composed")
        self.assertEqual(fs.get("/É"), b"combining")
        # A case-only rename keeps the entry and changes the stored name.
        fs.ok(SESSION, FS_RENAME, len("/Straße".encode()), data="/Straße/STRASSE")
        names = [n for n, _, _ in fs.list("/")]
        self.assertIn("STRASSE", names)
        self.assertNotIn("Straße", names)
        fs.ok(SESSION, FS_RENAME, len("/STRASSE".encode()), data="/STRASSE/STRASSE")
        self.assertEqual(fs.remount(), 0)


class DirectoryTests(FsTestBase):
    def test_root_listing_reserved_names_once(self):
        fs = self.fs
        image = fs.format()
        self.assertEqual(fs.attach(ROOT_MNT, image, alias=DD0), 0)
        # A disk entry named like a reserved name is hidden by the VFS.
        fs.ok(SESSION, FS_MKDIR, 0, DEFAULT, data="/dd/0/DEV")
        fs.ok(SESSION, FS_MKDIR, 0, DEFAULT, data="/home")
        names = sorted(n for n, _, _ in fs.list("/"))
        self.assertEqual(names, ["dd", "dev", "home", "os"])  # no RAM volume: no /tmp
        self.assertEqual(sorted(n for n, _, _ in fs.list("/dd/0")), ["DEV", "home"])
        self.assertEqual([n for n, _, _ in fs.list("/dd")], ["0"])
        self.assertEqual(sorted(n for n, _, _ in fs.list("/dev")), ["con", "fb0", "null", "sgu"])
        self.assertEqual(fs.list("/os"), [])
        st = stat_of(fs.ok(SESSION, FS_STAT, data="/DD").data)
        self.assertEqual((st["type"], st["mode"], st["flags"]), (2, 0x2D, 1))
        self.assertError("EXISTS", fs.call(SESSION, FS_MKDIR, 0, DEFAULT, data="/Tmp"))
        self.assertError("BUSY", fs.call(SESSION, FS_RMDIR, data="/os"))
        self.assertError("EXISTS", fs.call(SESSION, FS_RENAME, 5, data="/home/dev"))
        self.assertError("READ_ONLY", fs.call(SESSION, FS_MKDIR, 0, DEFAULT, data="/dev/x"))
        # "/" and /dd/0 are one volume: an open file blocks both names.
        fs.put("/home/f", b"x")
        h = fs.open("/dd/0/home/f")
        self.assertError("BUSY", fs.call(SESSION, FS_UNLINK, data="/home/f"))
        self.assertError("BUSY", fs.call(SESSION, FS_RENAME, 5, data="/home/h2"))
        fs.lib.host_gone(h)
        # .. at a mount root leaves the mount.
        fs.ok(SESSION, FS_CWD_SET, data="/dd/0/home/..")
        self.assertEqual(fs.ok(SESSION, FS_CWD_GET).data, b"/dd/0")
        fs.ok(SESSION, FS_CWD_SET, data="..")
        self.assertEqual(fs.ok(SESSION, FS_CWD_GET).data, b"/dd")

    def test_listing_split_by_capacity(self):
        fs = self.fs
        fs.boot_disk()
        expected = set()
        for i in range(40):
            name = f"file-{i:02d}-" + "x" * (i * 6)
            fs.put("/" + name, b"")
            expected.add(name)
        fs.ok(SESSION, FS_UNLINK, data="/file-05-" + "x" * 30)
        expected.discard("file-05-" + "x" * 30)
        for cap in (4096, 1096, 330):
            got = [n for n, _, _ in fs.list("/", cap=cap)]
            self.assertEqual(len(got), len(set(got)))
            self.assertEqual(set(got), expected | {"dev", "dd", "os", "tmp"})
        d = fs.ok(SESSION, FS_DIR_OPEN, data="/").new_badge
        self.assertError("LIMIT", fs.call(d, DIR_NEXT, cap=40))
        first = entries_of(fs.ok(d, DIR_NEXT, cap=330).data)
        fs.ok(d, DIR_REWIND)
        self.assertEqual(entries_of(fs.ok(d, DIR_NEXT, cap=330).data), first)
        self.assertError("UNSUPPORTED", fs.call(d, ST_READ, 1))

    def test_mkdir_rmdir_rename(self):
        fs = self.fs
        fs.boot_disk()
        fs.ok(SESSION, FS_MKDIR, 0, DEFAULT, data="/a")
        fs.ok(SESSION, FS_MKDIR, 0, DEFAULT, data="/a/b")
        fs.put("/a/b/f", b"hello")
        self.assertError("NOT_EMPTY", fs.call(SESSION, FS_RMDIR, data="/a"))
        self.assertError("NOT_DIR", fs.call(SESSION, FS_RMDIR, data="/a/b/f"))
        self.assertError("IS_DIR", fs.call(SESSION, FS_UNLINK, data="/a/b"))
        self.assertError("EXISTS", fs.call(SESSION, FS_MKDIR, 0, DEFAULT, data="/a"))
        self.assertError("BAD_ARG", fs.call(SESSION, FS_RENAME, 2, data="/a/a/b/c"))  # into itself
        self.assertError("BAD_PATH", fs.call(SESSION, FS_RMDIR, data="/a/."))
        self.assertError("BUSY", fs.call(SESSION, FS_RMDIR, data="/"))
        fs.ok(SESSION, FS_MKDIR, 0, DEFAULT, data="/z")
        fs.ok(SESSION, FS_RENAME, 4, data="/a/b/z/b")
        self.assertEqual(fs.get("/z/b/f"), b"hello")
        fs.put("/z/g", b"")
        self.assertError("EXISTS", fs.call(SESSION, FS_RENAME, 6, data="/z/b/f/z/g"))
        self.assertError("CROSS_DEVICE", fs.call(SESSION, FS_RENAME, 6, data="/z/b/f/tmp/f"))
        fs.ok(SESSION, FS_UNLINK, data="/z/b/f")
        fs.ok(SESSION, FS_RMDIR, data="/z/b")
        fs.ok(SESSION, FS_RMDIR, data="/a/")
        self.assertEqual(sorted(n for n, _, _ in fs.list("/z")), ["g"])
        self.assertEqual(fs.remount(), 0)

    def test_directory_index_follows_changes(self):
        # Lookups use a per-directory hash index that is dropped on every
        # change; more directories than index slots forces rebuilds, and a
        # freed slot must be the one a later create reuses.
        fs = self.fs
        fs.boot_disk()
        for d in "abcdefg":
            fs.ok(SESSION, FS_MKDIR, 0, DEFAULT, data=f"/{d}")
            fs.put(f"/{d}/one", d.encode())
            fs.put(f"/{d}/Two", d.encode() * 2)
        for _ in range(2):
            for d in "abcdefg":
                self.assertEqual(fs.get(f"/{d}/ONE"), d.encode())
                self.assertEqual(fs.get(f"/{d}/two"), d.encode() * 2)
        self.assertError("NOT_FOUND", fs.call(SESSION, FS_OPEN, READ, 0, data="/a/three"))
        fs.ok(SESSION, FS_UNLINK, data="/a/one")
        self.assertError("NOT_FOUND", fs.call(SESSION, FS_OPEN, READ, 0, data="/a/one"))
        fs.put("/a/three", b"3")
        self.assertEqual(fs.get("/a/THREE"), b"3")
        self.assertEqual(sorted(n for n, _, _ in fs.list("/a")), ["Two", "three"])
        fs.ok(SESSION, FS_RENAME, len("/a/three"), data="/a/three/b/moved")
        self.assertEqual(fs.get("/b/MOVED"), b"3")
        self.assertError("NOT_FOUND", fs.call(SESSION, FS_OPEN, READ, 0, data="/a/three"))
        self.assertEqual(fs.remount(), 0)

    def test_tmp_is_a_separate_ram_volume(self):
        fs = self.fs
        fs.boot_disk()
        fs.put("/tmp/x", b"ram")
        self.assertEqual(fs.get("/TMP/x"), b"ram")
        vi = fs.ok(SESSION, FS_VOLUME_INFO, data="/tmp").data
        self.assertEqual((be(vi, 0), be(vi, 4), be(vi, 24)), (3, 0, 0xFFFFFFFF))
        vi = fs.ok(SESSION, FS_VOLUME_INFO, data="/").data
        self.assertEqual((be(vi, 0), be(vi, 24)), (1, 0))
        self.assertEqual(be(vi, 8, 8), 508 * (2880 - 67))
        vi = fs.ok(SESSION, FS_VOLUME_INFO, data="/dev").data
        self.assertEqual((be(vi, 0), be(vi, 4)), (4, 1))


class DeviceTests(FsTestBase):
    def test_dev_nodes(self):
        fs = self.fs
        fs.boot_disk()
        r = fs.ok(SESSION, FS_OPEN, READ | WRITE, data="/dev/con")
        self.assertEqual((r.a[0], r.fwd, r.new_badge), (4, 77, 0))
        self.assertEqual(fs.lib.host_dev_open_flags(), READ | WRITE)
        fb = fs.ok(SESSION, FS_OPEN, READ | WRITE, data="/dev/fb0")
        self.assertEqual((fb.a[0], fb.fwd, fb.new_badge), (3, 77, 0))
        self.assertEqual(stat_of(fs.ok(SESSION, FS_STAT, data="/dev/fb0").data)["type"], 3)
        sgu = fs.ok(SESSION, FS_OPEN, READ | WRITE, data="/dev/sgu")
        self.assertEqual((sgu.a[0], sgu.fwd, sgu.new_badge), (3, 77, 0))
        self.assertEqual(stat_of(fs.ok(SESSION, FS_STAT, data="/dev/sgu").data)["type"], 3)
        # TRUNC and APPEND are ignored for devices ("> /dev/null"); EXEC is not.
        fs.ok(SESSION, FS_OPEN, WRITE | CREATE | TRUNC, DEFAULT, data="/dev/con")
        self.assertEqual(fs.lib.host_dev_open_flags(), WRITE)
        self.assertError("ACCESS", fs.call(SESSION, FS_OPEN, READ | EXEC, data="/dev/con"))
        self.assertError("EXISTS", fs.call(SESSION, FS_OPEN, WRITE | CREATE | EXCL, DEFAULT, data="/dev/null"))
        n = fs.ok(SESSION, FS_OPEN, READ | WRITE, data="/dev/null")
        self.assertEqual(n.a[0], 3)
        self.assertEqual(fs.ok(n.new_badge, ST_WRITE, data=b"gone").a[0], 4)
        self.assertEqual(fs.ok(n.new_badge, ST_READ, 10).data, b"")
        self.assertError("UNSUPPORTED", fs.call(n.new_badge, ST_SEEK))
        self.assertEqual(stat_of(fs.ok(n.new_badge, ST_STAT).data)["type"], 3)


class VolumeTests(FsTestBase):
    def sb_flags(self, mnt=ROOT_MNT):
        return be(self.image_bytes(mnt), 512 + 6, 2)

    def test_dirty_set_on_first_change_and_cleared_on_unmount(self):
        fs = self.fs
        image = fs.boot_disk()
        fs.list("/")
        self.assertEqual(self.sb_flags(), 0)  # reading leaves the volume clean
        fs.put("/x", b"1")
        self.assertEqual(self.sb_flags(), 1)
        self.assertEqual(s32(fs.lib.host_unmount(ROOT_MNT) & 0xFFFFFFFF), 0)
        self.assertEqual(self.sb_flags(), 0)
        image[512 + 7] = 1
        fs2 = Host(self.lib)
        self.assertEqual(fs2.attach(ROOT_MNT, image), E["NEEDS_FSCK"])

    def test_io_failure_stops_updates(self):
        fs = self.fs
        fs.boot_disk()
        fs.put("/x", b"1")
        fs.lib.host_fail_writes_after(ROOT_MNT, 2)
        h = fs.open("/x", WRITE)
        self.assertError("IO", fs.call(h, ST_WRITE, data=bytes(3000)))
        self.assertError("IO", fs.call(SESSION, FS_MKDIR, 0, DEFAULT, data="/y"))
        fs.lib.host_fail_writes_after(ROOT_MNT, 0xFFFFFFFF)
        self.assertError("IO", fs.call(SESSION, FS_MKDIR, 0, DEFAULT, data="/y"))
        self.assertEqual(self.sb_flags(), 1)

    def test_contract_example_space_and_inodes(self):
        # §12: 2880 sectors, 256 inodes: data_start 67, 255 inodes besides the root.
        fs = self.fs
        image = fs.format(sectors=2880, inodes=256)
        self.assertEqual(be(bytes(image), 512 + 0x2C), 67)
        self.assertEqual(fs.attach(ROOT_MNT, image, alias=DD0), 0)
        for i in range(255):
            fs.lib.host_gone(fs.open(f"/{i}", WRITE | CREATE, DEFAULT))
        self.assertError("LIMIT", fs.call(SESSION, FS_OPEN, WRITE | CREATE, DEFAULT, data="/full"))
        self.assertEqual(fs.remount(), 0)
        # A full disk: the first create+write needs room for both.
        tiny = fs.format(sectors=9, inodes=4)  # data blocks 4..8: 5 blocks
        self.assertEqual(fs.attach(DD0 + 1, tiny, slot=1), 0)
        h = fs.ok(SESSION, FS_OPEN, WRITE | CREATE, DEFAULT, data="/dd/1/a").new_badge
        self.assertEqual(fs.ok(h, ST_WRITE, data=bytes(600)).a[0], 600)  # 3 + 2 blocks
        fs.ok(h, ST_WRITE, data=bytes(1016 - 600))
        self.assertError("NO_SPACE", fs.call(h, ST_WRITE, data=b"x"))
        self.assertError("NO_SPACE", fs.call(SESSION, FS_MKDIR, 0, DEFAULT, data="/dd/1/b"))

    def test_read_only_mount(self):
        fs = self.fs
        image = fs.format()
        self.assertEqual(fs.attach(ROOT_MNT, image, writable=False), 0)
        self.assertError("READ_ONLY", fs.call(SESSION, FS_MKDIR, 0, DEFAULT, data="/a"))
        self.assertError("READ_ONLY", fs.call(SESSION, FS_OPEN, WRITE | CREATE, DEFAULT, data="/a"))
        st = stat_of(fs.ok(SESSION, FS_STAT, data="/").data)
        self.assertEqual(st["flags"], 1)

    def test_mount_rejects_corruption(self):
        fs = self.fs
        good = fs.format(sectors=200, inodes=16)
        fs.attach(ROOT_MNT, good)
        fs.ok(SESSION, FS_MKDIR, 0, DEFAULT, data="/d")
        fs.put("/d/f", bytes(range(200)) * 5)
        fs.lib.host_unmount(ROOT_MNT)
        base = bytes(good)
        sb = 512
        layout = dict(bitmap=be(base, sb + 0x18) * 512, inodes=be(base, sb + 0x20) * 512,
                      data=be(base, sb + 0x2C))

        def mounted(mutate, expect):
            image = (ctypes.c_uint8 * len(base)).from_buffer_copy(base)
            mutate(image)
            self.assertEqual(Host(self.lib).attach(ROOT_MNT, image), E[expect])

        self.assertEqual(Host(self.lib).attach(ROOT_MNT, (ctypes.c_uint8 * len(base)).from_buffer_copy(base)), 0)

        def setb(off, value):
            return lambda m: m.__setitem__(off, value)

        mounted(setb(0x1FF, 0), "FORMAT")                 # boot signature
        mounted(setb(sb + 0, ord("X")), "FORMAT")         # magic
        mounted(setb(sb + 5, 2), "UNSUPPORTED")           # version
        mounted(setb(sb + 6 + 1, 2), "UNSUPPORTED")       # unknown flag
        mounted(setb(sb + 0x37, 1), "UNSUPPORTED")        # casefold version
        mounted(setb(sb + 0x100, 1), "FORMAT")            # reserved
        mounted(setb(sb + 0x2F, 99), "FORMAT")            # data_start
        mounted(setb(layout["bitmap"], 0x7F), "FORMAT")   # metadata bit clear
        last = layout["bitmap"] + 199 // 8
        mounted(lambda m: m.__setitem__(last, m[last] | 1), "FORMAT")  # bit past total_blocks
        ino2 = layout["inodes"] + 128
        mounted(setb(ino2 + 1, 1), "FORMAT")              # inode flags
        mounted(lambda m: (m.__setitem__(ino2 + 0x28, 3), m.__setitem__(ino2 + 0x29, 0xE8)), "FORMAT")  # ms 1000
        mounted(setb(ino2 + 0x17, 1), "FORMAT")           # dir size not a multiple
        free_ino = layout["inodes"] + 128 * 10
        mounted(setb(free_ino + 5, 1), "FORMAT")          # free inode not zero
        # A data block marked used but owned by no chain (a leak).
        data_bit = layout["bitmap"] + 190 // 8
        mounted(lambda m: m.__setitem__(data_bit, m[data_bit] | (0x80 >> (190 % 8))), "FORMAT")


class QuickMountTests(FsTestBase):
    """The temporary guest mode still initialises allocation state correctly."""
    mount_cflags = ["-DFS2_FULL_MOUNT_CHECK=0"]

    def test_existing_files_allocation_and_remount(self):
        fs = self.fs
        fs.boot_disk()
        fs.put("/old", b"old" * 400)
        self.assertEqual(fs.remount(), 0)
        fs.put("/new", b"new" * 700)
        self.assertEqual(fs.get("/old"), b"old" * 400)
        self.assertEqual(fs.get("/new"), b"new" * 700)
        self.assertEqual(fs.remount(), 0)
        self.assertEqual(fs.get("/new"), b"new" * 700)
        self.assertEqual(fs2tool.check_image(bytearray(self.image_bytes())), [])

    def test_skips_chain_scan_but_keeps_dirty_and_layout_checks(self):
        fs = self.fs
        good = fs.format(sectors=200, inodes=16)
        fs.attach(ROOT_MNT, good)
        fs.put("/f", b"payload")
        fs.lib.host_unmount(ROOT_MNT)
        base = bytes(good)
        inodes = be(base, 512 + 0x20) * 512
        first = be(base, inodes + 128 + 0x0C)
        # Nonzero payload padding is rejected by a full scan, but not by
        # the temporary guest mode. Mount must not write to the medium.
        image = (ctypes.c_uint8 * len(base)).from_buffer_copy(base)
        image[first * 512 + 511] = 1
        before = bytes(image)
        self.assertEqual(Host(self.lib).attach(ROOT_MNT, image), 0)
        self.assertEqual(bytes(image), before)
        self.assertTrue(fs2tool.check_image(bytearray(image)))
        for offset, value, error in ((512 + 7, 1, "NEEDS_FSCK"),
                                     (512 + 0x2F, 99, "FORMAT"),
                                     (inodes + 1, 1, "FORMAT")):
            image = (ctypes.c_uint8 * len(base)).from_buffer_copy(base)
            image[offset] = value
            before = bytes(image)
            self.assertEqual(Host(self.lib).attach(ROOT_MNT, image), E[error])
            self.assertEqual(bytes(image), before)


class DsfsTests(FsTestBase):
    def test_dsfs_slot_is_read_only(self):
        fs = self.fs
        fs.boot_disk()
        img = bytearray(2880 * 512)
        dsfs.mkfs(img, created=0)
        dsfs.put_file(img, dsfs.normalize_name("HELLO.TXT"), b"hello dsfs" * 100, 1234)
        dsfs.put_file(img, dsfs.normalize_name("README"), b"", 99)
        image = (ctypes.c_uint8 * len(img)).from_buffer(img)
        self.assertEqual(s32(self.lib.host_attach_dsfs(DD0 + 2, image, 2880, 2) & 0xFFFFFFFF), 0)
        entries = {n: st for n, _, st in fs.list("/dd/2")}
        self.assertEqual(set(entries), {"HELLO.TXT", "README"})
        self.assertEqual((entries["HELLO.TXT"]["size"], entries["HELLO.TXT"]["mode"]), (1000, 0x24))
        self.assertEqual(entries["README"]["modified"], (99, 0))
        self.assertEqual(fs.get("/dd/2/hello.txt"), b"hello dsfs" * 100)
        h = fs.open("/dd/2/HELLO.TXT")
        fs.ok(h, ST_SEEK, 2, 0xFFFFFFFF, 0xFFFFFFFC)
        self.assertEqual(fs.ok(h, ST_READ, 100).data, b"dsfs")
        self.assertError("READ_ONLY", fs.call(SESSION, FS_OPEN, WRITE, data="/dd/2/HELLO.TXT"))
        self.assertError("READ_ONLY", fs.call(SESSION, FS_UNLINK, data="/dd/2/README"))
        self.assertError("READ_ONLY", fs.call(SESSION, FS_OPEN, WRITE | CREATE, DEFAULT, data="/dd/2/NEW"))
        self.assertError("NOT_DIR", fs.call(SESSION, FS_STAT, data="/dd/2/README/x"))
        vi = fs.ok(SESSION, FS_VOLUME_INFO, data="/dd/2").data
        self.assertEqual((be(vi, 0), be(vi, 4), be(vi, 24)), (2, 1, 2))
        self.assertEqual(be(vi, 16, 8), 512 * (2861 - 2))
        self.assertEqual(sorted(n for n, _, _ in fs.list("/dd")), ["0", "2"])
        # §7: a broken DSFS volume is not mounted.
        img[512 * 3] = 2
        bad = (ctypes.c_uint8 * len(img)).from_buffer(img)
        self.assertEqual(s32(self.lib.host_attach_dsfs(DD0 + 3, bad, 2880, 3) & 0xFFFFFFFF), E["FORMAT"])


@unittest.skipIf(fs2tool is None, "tools/fs2/fs2.py is not available")
class Fs2ToolTests(FsTestBase):
    """The guest driver and the host tool on the same images (§12 step 2)."""

    FILES = {
        "/bin/ls.wax": bytes(range(256)) * 9,
        "/bin/empty": b"",
        "/home/ユーザー/メモ.txt": "日本語のテキスト".encode() * 70,
        "/home/Straße/x": b"x",
        "/etc/profile.fsh": b"echo hi\n",
        "/edge/508": bytes([7]) * 508,
        "/edge/509": bytes([9]) * 509,
    }

    def host_image(self):
        img = fs2tool.format_image(2880, inode_count=128, created=(1_600_000_000, 5))
        vol = fs2tool.Volume(img)
        vol.mkdir("/junk", stamp=(1, 0))
        for i in range(30):  # fragment the free space
            vol.put(f"/junk/{i}", bytes(600 * (i % 3)), stamp=(1, 0))
        for path, data in self.FILES.items():
            vol.mkdir(path.rsplit("/", 1)[0] or "/", parents=True, stamp=(2, 0))
            vol.put(path, data, stamp=(3, 999))
        for i in range(0, 30, 2):
            vol.remove(f"/junk/{i}", stamp=(4, 0))
        vol.put("/bin/big", bytes(range(251)) * 40, stamp=(5, 0))
        self.assertEqual(fs2tool.check_image(img), [])
        return img

    def attach_bytes(self, img):
        image = (ctypes.c_uint8 * len(img)).from_buffer(img)
        self.assertEqual(self.fs.attach(ROOT_MNT, image, alias=DD0), 0)
        return image

    def test_guest_reads_host_image(self):
        img = self.host_image()
        fs = self.fs
        self.attach_bytes(img)
        for path, data in self.FILES.items():
            self.assertEqual(fs.get(path), data, path)
            self.assertEqual(fs.get(path.upper()), data, path)
        self.assertEqual(fs.get("/bin/big"), bytes(range(251)) * 40)
        st = stat_of(fs.ok(SESSION, FS_STAT, data="/edge/509").data)
        self.assertEqual((st["modified"], st["mode"]), ((3, 999), 0x34))
        self.assertEqual(sorted(n for n, _, _ in fs.list("/junk")), sorted(str(i) for i in range(1, 30, 2)))
        self.assertEqual(be(bytes(img), 512 + 6, 2), 0)  # reading leaves it clean

    def test_host_checks_guest_changes(self):
        img = self.host_image()
        fs = self.fs
        self.attach_bytes(img)
        fs.put("/bin/ls.wax", b"replaced")
        fs.put("/new file", bytes(range(200)) * 13)
        fs.ok(SESSION, FS_MKDIR, 0, 0x3F, data="/home/ユーザー/sub")
        fs.ok(SESSION, FS_RENAME, len("/edge/509"), data="/edge/509/home/ユーザー/sub/moved".encode())
        fs.ok(SESSION, FS_RENAME, len("/home/straße".encode()), data="/home/straße/home/STRASSE".encode())
        fs.ok(SESSION, FS_UNLINK, data="/junk/1")
        h = fs.open("/bin/big", READ | WRITE)
        fs.ok(h, ST_TRUNCATE, 0, 0, 1000)
        fs.ok(h, ST_SEEK, 0, 0, 5000)
        fs.write(h, b"tail")
        fs.lib.host_gone(h)
        fs.ok(SESSION, FS_CHMOD, 0x24, data="/etc/profile.fsh")
        fs.ok(SESSION, FS_CHOWN, 1000, data="/home/ユーザー")
        self.assertEqual(be(bytes(img), 512 + 6, 2), 1)
        self.assertEqual(fs2tool.check_image(img, allow_dirty=True), [])
        self.assertEqual(s32(fs.lib.host_unmount(ROOT_MNT) & 0xFFFFFFFF), 0)
        self.assertEqual(fs2tool.check_image(img), [])
        vol = fs2tool.Volume(img)
        self.assertEqual(vol.read(vol.resolve("/bin/ls.wax")), b"replaced")
        self.assertEqual(vol.read(vol.resolve("/new file")), bytes(range(200)) * 13)
        self.assertEqual(vol.read(vol.resolve("/home/ユーザー/sub/moved")), bytes([9]) * 509)
        self.assertEqual(vol.read(vol.resolve("/bin/big")), (bytes(range(251)) * 40)[:1000] + bytes(4000) + b"tail")
        self.assertEqual(vol.resolve("/etc/profile.fsh")["mode"], 0x24)
        self.assertEqual(vol.resolve("/home/ユーザー")["uid"], 1000)
        names = [n for _, _, n in vol.entries(vol.resolve("/home"))]
        self.assertIn("STRASSE", names)
        self.assertNotIn("/junk/1", [n for _, _, n in vol.entries(vol.resolve("/junk"))])

    def test_same_verdict_on_corruption(self):
        base = bytes(self.host_image())
        sb = 512
        inodes = be(base, sb + 0x20) * 512
        bitmap = be(base, sb + 0x18) * 512
        mutations = {
            "magic": (sb, 0x46 ^ 1),
            "dirty": (sb + 7, 1),
            "inode type": (inodes + 128 * 3, 9),
            "root parent": (inodes + 11, 2),
            "leaked block": (bitmap + 2879 // 8, 0xFF),
            "ms": (inodes + 128 * 4 + 0x34, 0x10),
        }
        for label, (offset, value) in mutations.items():
            img = bytearray(base)
            img[offset] = value
            host = fs2tool.check_image(img)
            guest = Host(self.lib).attach(ROOT_MNT, (ctypes.c_uint8 * len(img)).from_buffer(img))
            self.assertTrue(host, label)
            self.assertLess(guest, 0, label)

    def test_casefold_tables_agree(self):
        # Every C/F mapping of the shared table: a file named by the source
        # scalar is found again through its folded form.
        items = sorted(fs2tool.CASEFOLD.items())
        img = fs2tool.format_image(16384, inode_count=2 * len(items) + 8)
        vol = fs2tool.Volume(img)
        for i, (cp, _) in enumerate(items):
            vol.mkdir(f"/{i}", stamp=(0, 0))
            vol.put(f"/{i}/" + chr(cp), b"%d" % cp, stamp=(0, 0))
        fs = self.fs
        self.attach_bytes(img)
        for i, (cp, folded) in enumerate(items):
            self.assertEqual(fs.get(f"/{i}/" + folded), b"%d" % cp, hex(cp))


class NameTests(FsTestBase):
    def test_libfnd_name_functions(self):
        lib = self.lib
        lib.host_name_equal.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
        lib.host_name_match.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
        eq = lambda a, b: lib.host_name_equal(a.encode(), b.encode())  # noqa: E731
        match = lambda p, n: lib.host_name_match(p.encode(), n.encode())  # noqa: E731
        self.assertEqual(eq("Straße", "STRASSE"), 1)
        self.assertEqual(eq("é", "é"), 0)
        self.assertEqual(eq("Readme", "README"), 1)
        self.assertEqual(match("*.TXT", "readme.txt"), 1)
        self.assertEqual(match("r?adme.*", "README.md"), 1)
        self.assertEqual(match("*.txt", "readme.md"), 0)
        self.assertEqual(match("a*b*c", "aXbYbZc"), 1)
        self.assertEqual(lib.host_name_equal(b"\xff", b"\xff"), 0)
        out = ctypes.create_string_buffer(16)
        n = ctypes.c_uint32()
        lib.host_name_fold.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint32, ctypes.POINTER(ctypes.c_uint32)]
        self.assertEqual(lib.host_name_fold("Straße".encode(), out, 16, ctypes.byref(n)), 0)
        self.assertEqual(out.value, b"strasse")
        self.assertEqual(s32(lib.host_name_fold(b"ABCDEFGHIJKLMNOPQ", out, 16, ctypes.byref(n)) & 0xFFFFFFFF),
                         E["LIMIT"])
        self.assertEqual(n.value, 18)


if __name__ == "__main__":
    unittest.main()
