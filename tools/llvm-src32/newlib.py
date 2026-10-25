"""Optional source-only Newlib integration. Never downloads or vendors Newlib."""
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess

PORT = Path(__file__).resolve().parent / "runtime" / "newlib"
STRINGS = "memcpy memmove memset memcmp memchr strlen strcpy strncpy strcat strncat strcmp strncmp strchr strrchr strstr strspn strcspn strpbrk".split()
INTEGERS = "abs labs llabs div ldiv lldiv".split()
SOURCES = {name: f"string/{name}.c" for name in STRINGS}
SOURCES.update({name: f"stdlib/{name}.c" for name in INTEGERS})
SOURCES.update({name: f"search/{name}.c" for name in ("qsort", "bsearch")})
# The "posix" profile (doc/spec_pkcl.md §9): most of Newlib's libc, compiled on demand
# from a symbol index. nano-* are the size-reduced variants of printf, which
# the full ones replace; posix/, sys/, machine/ and friends are OS or CPU glue
# that pkcl (foundation/lib/pkcl) provides instead.
POSIX_DIRS = ("libc/argz", "libc/ctype", "libc/errno", "libc/locale", "libc/misc", "libc/reent",
              "libc/search", "libc/signal", "libc/stdio", "libc/stdlib", "libc/string", "libc/time",
              "libm/common", "libm/math")
POSIX_EXCLUDE = ("mallocr", "malloc.c", "mlock", "nano-mallocr", "sbrkr", "mstats", "malloc_r",
                 "/stdlib/cxa_", "/stdlib/callocr.c", "/stdlib/cfreer.c", "/stdlib/freer.c", "/stdlib/malignr.c",
                 "/stdlib/mallinfor.c", "/stdlib/malloptr.c", "/stdlib/mallstatsr.c", "/stdlib/msizer.c",
                 "/stdlib/pvallocr.c", "/stdlib/realloc.c", "/stdlib/calloc.c", "/stdlib/free.c", "/stdlib/valloc.c", "/stdlib/pvalloc.c", "/stdlib/memalign.c", "/stdlib/mallinfo.c", "/stdlib/malloc_usable_size.c", "/stdlib/reallocr.c", "/stdlib/vallocr.c", "/stdlib/mallocr.c", "/misc/ffs", "fdopen64", "/stdio64/",
                 # the driver's own string.c supplies these (it compiles them at -O2)
                 "/string/memcpy.c", "/string/memmove.c", "/string/memset.c", "/string/memcmp.c", "/string/strlen.c")
# Files that Newlib compiles as variants of another file; compile only the first of a group.
POSIX_PRIORITY = ("stdio/nano-vfprintf.c", "stdio/nano-svfprintf.c", "stdio/nano-vfscanf.c", "stdio/nano-svfscanf.c",
                  "stdio/nano-vfprintf_i.c", "stdio/nano-vfscanf_i.c")
POSIX_DEFINES = ("_COMPILING_NEWLIB", "_GNU_SOURCE")
CACHE_VERSION = "5"  # 2: the library is compiled with -Os (it was -O0)
REENT_ALLOCATORS = ("_malloc_r", "_free_r", "_realloc_r", "_calloc_r", "__malloc_lock", "__malloc_unlock")
ALLOCATORS = ("malloc", "free", "calloc", "realloc", "malloc_usable_size")


_ALIAS = re.compile(r"^@([\w.$]+) = (?:[a-z_]+ )*alias (\w+) \(([^)]*)\), ptr @([\w.$]+)$", re.M)


def functions_for_aliases(text):
    """The backend has no aliases or weak symbols. Newlib's nano printf and scanf make the integer-only entry points
    (vfiprintf and the like) aliases of the full ones, so define them as forwarding functions."""
    def forward(match):
        name, result, parameters, target = match.groups()
        types = [t.strip() for t in parameters.split(",") if t.strip()]
        names = ", ".join(f"{t} %a{i}" for i, t in enumerate(types))
        call = f"@{target}({names})"
        if result == "void":
            return f"define dso_local void @{name}({names}) {{\n  tail call void {call}\n  ret void\n}}"
        return f"define dso_local {result} @{name}({names}) {{\n  %r = tail call {result} {call}\n  ret {result} %r\n}}"
    text = _ALIAS.sub(forward, text)
    # nano printf and scanf reach the floating-point code through weak references that the linker
    # leaves null when it is absent. There are no weak symbols here: link the code in, always.
    return re.sub(r"^(define|declare) (?:extern_weak|weak) ", r"\1 ", text, flags=re.M)


class Newlib:
    def __init__(self, source, clang, heap_size, profile="portable", compile_ir=None, jobs=1, cache=None):
        self.root = Path(source).resolve()
        self.libc = self.root / "newlib" / "libc"
        required = [self.libc / "include" / "newlib.h", self.root / "COPYING.NEWLIB",
                    *(self.libc / path for path in SOURCES.values()),
                    self.libc / "stdlib" / "nano-mallocr.c"]
        for path in required:
            if not path.is_file():
                raise ValueError(f"invalid --newlib-source: missing {path}")
        if not 8 <= heap_size <= 0x7fffffff or heap_size % 8:
            raise ValueError("--newlib-heap-size must be a multiple of 8 in 8..0x7ffffff8")
        result = subprocess.run([clang, "-print-resource-dir"], capture_output=True,
                                text=True, check=True)
        self.resource = Path(result.stdout.strip()) / "include"
        if not (self.resource / "stddef.h").is_file():
            raise ValueError(f"Clang resource headers not found: {self.resource}")
        self.heap_size = heap_size
        self.loaded = set()
        self.profile = profile
        self.posix = profile == "posix"
        self.compile_ir = compile_ir
        self.jobs = jobs
        self.cache_root = cache
        self.index = None
        self.clang = clang

    def headers(self):
        # Port configuration, upstream declarations, then compiler-owned headers.
        return [PORT / ("include-posix" if self.posix else "include"), self.libc / "include", self.resource]

    # ---- posix profile: a cache of the IR of every library file, and the symbol index ----

    def candidates(self):
        files = []
        for directory in POSIX_DIRS:
            for path in sorted((self.libc.parent / directory).glob("*.c")):
                relative = path.relative_to(self.libc.parent).as_posix()
                if any(mark in "/" + relative or path.name.startswith(mark) for mark in POSIX_EXCLUDE):
                    continue
                files.append(path)
        return files

    def cache_dir(self):
        version = subprocess.run([self.clang, "--version"], capture_output=True, text=True).stdout
        digest = hashlib.sha256()
        for part in (CACHE_VERSION, str(self.root), version, self.profile, repr(POSIX_DIRS), repr(POSIX_EXCLUDE),
                     repr(POSIX_DEFINES)):
            digest.update(part.encode())
        for path in sorted((PORT / "include-posix").glob("*.h")):
            digest.update(path.read_bytes())
        base = Path(self.cache_root) if self.cache_root else Path(
            os.environ.get("SRC32_CACHE") or Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / "llvm-src32")
        return base / f"newlib-{digest.hexdigest()[:16]}"

    def build_index(self):
        """Compile every candidate to IR once (cached) and map defined symbols to their IR file."""
        from llvmlite import binding as llvm
        directory = self.cache_dir()
        index_file = directory / "index.json"
        if index_file.is_file():
            self.index = json.loads(index_file.read_text(encoding="utf-8"))
            return
        directory.mkdir(parents=True, exist_ok=True)
        files = self.candidates()

        def build(path):
            ir = directory / (path.relative_to(self.libc.parent).as_posix().replace("/", "-")[:-2] + ".ll")
            if not ir.is_file():
                try:
                    self.compile_ir(path, ir)
                    ir.write_text(functions_for_aliases(ir.read_text(encoding="utf-8")), encoding="utf-8")
                except Exception as exc:  # a file the toolchain cannot compile just provides nothing
                    return path, None, str(exc).splitlines()[0]
            return path, ir, None

        with ThreadPoolExecutor(max_workers=self.jobs) as pool:
            built = list(pool.map(build, files))
        symbols, failed = {}, {}
        for path, ir, error in built:
            relative = path.relative_to(self.libc.parent).as_posix()
            if ir is None:
                failed[relative] = error
                continue
            module = llvm.parse_assembly(ir.read_text(encoding="utf-8"))
            names = [f.name for f in module.functions if not f.is_declaration and str(f.linkage) not in
                     ("Linkage.internal", "Linkage.private")]
            names += [g.name for g in module.global_variables if not g.is_declaration and str(g.linkage) not in
                      ("Linkage.internal", "Linkage.private")]
            for name in names:
                entry = symbols.setdefault(name, [])
                entry.append(ir.name)
        for name, entry in symbols.items():
            entry.sort(key=lambda item: (0 if any(item.endswith(p.replace("/", "-")[:-2] + ".ll")
                                                  for p in POSIX_PRIORITY) else 1, item))
        data = {"symbols": {k: v[0] for k, v in symbols.items()}, "failed": failed}
        index_file.write_text(json.dumps(data, indent=0, sort_keys=True), encoding="utf-8")
        self.index = data

    def posix_pending(self, missing, defined):
        if self.index is None:
            self.build_index()
        directory = self.cache_dir()
        result = []
        for name in sorted(missing):
            ir = self.index["symbols"].get(name)
            if ir and ir not in self.loaded:
                self.loaded.add(ir)
                result.append((ir[:-3], None, [], directory / ir))
        return result

    def pending(self, missing, defined=()):
        result = self.posix_pending(missing, defined) if self.posix else []
        for name in sorted(missing):
            if not self.posix and name in SOURCES and name not in self.loaded:
                self.loaded.add(name)
                result.append((name, self.libc / SOURCES[name], []))
        if (set(ALLOCATORS) | set(REENT_ALLOCATORS)) & missing and "allocator" not in self.loaded:
            if set(ALLOCATORS) & set(defined):
                raise ValueError("Newlib nano allocator cannot be mixed with user-defined malloc/free/realloc/calloc/malloc_usable_size")
            self.loaded.add("allocator")
            # Compile upstream nano malloc in its documented standalone mode.
            result.append(("allocator", self.libc / "stdlib/nano-mallocr.c",
                           ["DEFINE_MALLOC", "DEFINE_FREE", "DEFINE_CALLOC", "DEFINE_REALLOC",
                            "DEFINE_MALLOC_USABLE_SIZE"]))
        return result + self.port_pending(missing)

    def port_pending(self, missing):
        # errno and a bounded heap, unless the program (pkcl on Foundation) brings its own.
        result = []
        # (the posix profile has Newlib's own __errno, which keeps errno in the reent structure)
        wanted = {"cpt32_newlib_sbrk"} if self.posix else {"__errno", "cpt32_newlib_sbrk"}
        if wanted & missing and "port" not in self.loaded:
            self.loaded.add("port")
            defines = [f"CPT32_NEWLIB_HEAP_SIZE={self.heap_size}"] + (["CPT32_NEWLIB_NO_ERRNO"] if self.posix else [])
            result.append(("port", PORT / "port.c", defines))
        # Newlib's own code calls the reentrant forms of the allocator.
        if self.posix and missing & set(REENT_ALLOCATORS) and "reent-alloc" not in self.loaded:
            self.loaded.add("reent-alloc")
            result.append(("reent-alloc", PORT / "reent_alloc.c", []))
        return result
