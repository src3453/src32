"""Local cache for the individual compiler outputs used by the boot builder."""
from importlib.metadata import version
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import zipfile

TOOLCHAIN = Path(__file__).resolve().parents[2] / "tools/llvm-src32"
sys.path.insert(0, str(TOOLCHAIN))
import driver as compiler


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def atomic_write(path, write):
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as stream:
        temporary = Path(stream.name)
    try:
        write(temporary)
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


class ArtifactCache:
    def __init__(self, directory, clang):
        self.directory = directory.resolve()
        self.directory.mkdir(parents=True, exist_ok=True)
        try:
            self.clang = Path(compiler.find_clang(clang)).absolute()
        except compiler.DriverError as error:
            raise ValueError(str(error)) from error
        tools = [self.clang, Path(__file__), *TOOLCHAIN.glob("*.py"),
                 TOOLCHAIN.parent / "asm/asm.py", TOOLCHAIN.parent / "wax/wax.py",
                 TOOLCHAIN.parent / "wax/wax3.py"]
        launcher = os.environ.get("SRC32_CLANG_LAUNCHER")
        if launcher:
            executable = shutil.which(launcher)
            if not executable:
                raise ValueError(f"Clang launcher not found: {launcher!r} (SRC32_CLANG_LAUNCHER)")
            tools.append(Path(executable))
        tools.extend(self.clang.parent.glob("*.cfg"))
        self.identity = [
            sys.version, version("llvmlite"), os.environ.get("SOURCE_DATE_EPOCH"),
            [(str(path), digest(path)) for path in sorted(tools)],
        ]
        self.parser = compiler.parser()

    def run(self, command, output):
        # Extra compiler search paths or driver overrides can introduce inputs
        # outside Foundation's known include search paths.
        if any(os.environ.get(name) for name in (
            "CPATH", "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH", "CCC_OVERRIDE_OPTIONS",
            "CLANG_CONFIG_FILE_SYSTEM_DIR", "CLANG_CONFIG_FILE_USER_DIR",
        )):
            subprocess.run(command, check=True)
            return

        flags = list(map(str, command[2:]))
        options = self.parser.parse_args(flags)
        flags[flags.index("-o") + 1] = output.name
        # Track additions/removals in search directories, including a new
        # header that takes precedence over one in the previous depfile.
        include_dirs = [Path(path) for path in options.includes]
        include_dirs.extend([TOOLCHAIN / "runtime/include", TOOLCHAIN / "runtime/softfloat"])
        names = {str(path) for directory in include_dirs for path in directory.rglob("*") if path.is_file()}
        names.update(str(path) for directory in {p.parent for p in options.inputs}
                     for path in directory.iterdir() if path.is_file())
        key = hashlib.sha256(json.dumps(
            [self.identity, str(Path.cwd()), flags, sorted(names)], ensure_ascii=True,
        ).encode()).hexdigest()
        entry = self.directory / (key + ".zip")
        if self.restore(entry, output):
            print(f"Reused {output}", flush=True)
            return

        started = time.time_ns()
        with tempfile.TemporaryDirectory() as work:
            depfile = Path(work) / "artifact.d"
            subprocess.run([*command, "--clang", str(self.clang), "--depfile", str(depfile)], check=True)
            try:
                dependencies = {}
                for path in compiler.read_dependencies(depfile):
                    before = path.stat()
                    contents = path.read_bytes()
                    if (before.st_mtime_ns >= started or path.stat().st_mtime_ns != before.st_mtime_ns
                            or any(token in contents for token in (b"__DATE__", b"__TIME__", b"__TIMESTAMP__"))):
                        return
                    dependencies[str(path)] = hashlib.sha256(contents).hexdigest()

                def save(temporary):
                    with zipfile.ZipFile(temporary, "w") as archive:
                        archive.writestr("dependencies.json", json.dumps(dependencies))
                        archive.write(output, "artifact")
                atomic_write(entry, save)
            except OSError as error:
                print(f"Could not cache {output}: {error}", file=sys.stderr)

    @staticmethod
    def restore(entry, output):
        try:
            with zipfile.ZipFile(entry) as archive:
                dependencies = json.loads(archive.read("dependencies.json"))
                if not isinstance(dependencies, dict) or not dependencies:
                    return False
                if any(digest(Path(path)) != expected for path, expected in dependencies.items()):
                    return False
                artifact = archive.read("artifact")
        except (OSError, ValueError, KeyError, zipfile.BadZipFile):
            return False
        atomic_write(output, lambda path: path.write_bytes(artifact))
        return True
