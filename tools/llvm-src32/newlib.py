"""Optional source-only Newlib integration. Never downloads or vendors Newlib."""
from pathlib import Path
import subprocess

PORT = Path(__file__).resolve().parent / "runtime" / "newlib"
STRINGS = "memcpy memmove memset memcmp memchr strlen strcpy strncpy strcat strncat strcmp strncmp strchr strrchr strstr strspn strcspn strpbrk".split()
INTEGERS = "abs labs llabs div ldiv lldiv".split()
SOURCES = {name: f"string/{name}.c" for name in STRINGS}
SOURCES.update({name: f"stdlib/{name}.c" for name in INTEGERS})
SOURCES.update({name: f"search/{name}.c" for name in ("qsort", "bsearch")})
ALLOCATORS = ("malloc", "free", "calloc", "realloc", "malloc_usable_size")


class Newlib:
    def __init__(self, source, clang, heap_size):
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

    def headers(self):
        # Port configuration, upstream declarations, then compiler-owned headers.
        return [PORT / "include", self.libc / "include", self.resource]

    def pending(self, missing, defined=()):
        result = []
        for name in sorted(missing):
            if name in SOURCES and name not in self.loaded:
                self.loaded.add(name)
                result.append((name, self.libc / SOURCES[name], []))
        if set(ALLOCATORS) & missing and "allocator" not in self.loaded:
            if set(ALLOCATORS) & set(defined):
                raise ValueError("Newlib nano allocator cannot be mixed with user-defined malloc/free/realloc/calloc/malloc_usable_size")
            self.loaded.add("allocator")
            # Compile upstream nano malloc in its documented standalone mode.
            result.append(("allocator", self.libc / "stdlib/nano-mallocr.c",
                           ["DEFINE_MALLOC", "DEFINE_FREE", "DEFINE_CALLOC", "DEFINE_REALLOC",
                            "DEFINE_MALLOC_USABLE_SIZE"]))
        if "__errno" in missing or "cpt32_newlib_sbrk" in missing:
            if "port" not in self.loaded:
                self.loaded.add("port")
                result.append(("port", PORT / "port.c",
                               [f"CPT32_NEWLIB_HEAP_SIZE={self.heap_size}"]))
        return result
