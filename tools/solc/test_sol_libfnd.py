"""Real libfnd/sol integration, including kernel, IPC services and WAL binding."""
import os
from pathlib import Path
import subprocess
import sys

import pytest

from build_foundation import build
from driver import DriverError, find_clang
from sol_compiler import compile_to_src32_asm
from sol_vm import compile_program
from test_sol_foundation import _testbench, _harness_asm

ROOT = Path(__file__).resolve().parents[2]
LIB = ROOT / "sol/foundation/libfnd.sol"


def test_libfnd_declarations_compile_without_language_changes():
    text = '!const FND_LIB_ENTRY 0x900000\n' + LIB.read_text(encoding="utf-8")
    program = compile_program(text, source_path=str(LIB), remove_unused_functions=False)
    assert program.abi == "Foundation"
    # Includes the full wrapper set, including operations the sample does not call.
    compile_to_src32_asm(text, source_path=str(LIB), remove_unused_functions=False)


@pytest.mark.parametrize("origin", [0, 0x600000, 0x800004, 0x1000000])
def test_libfnd_builder_rejects_slots_that_cannot_hold_it(tmp_path, origin):
    with pytest.raises(ValueError, match="slot 5.*slot 6"):
        build(ROOT / "examples/foundation-sol/main.sol", tmp_path / "bad.wax", org=origin)


@pytest.fixture(scope="module")
def guest_image(tmp_path_factory):
    harness = _testbench()
    pytest.importorskip("llvmlite")
    try:
        clang = find_clang(os.environ.get("CLANG"))
    except DriverError as exc:
        pytest.skip(str(exc))
    work = tmp_path_factory.mktemp("sol-libfnd")
    result = subprocess.run([sys.executable, str(ROOT / "examples/foundation-sol/build.py"),
                             str(work), "--clang", clang, "--stop-ticks", "40000"],
                            capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=600)
    assert result.returncode == 0, result.stdout + result.stderr
    return harness, work


@pytest.mark.parametrize("cpu", ["scalar", "pipeline"])
def test_libfnd_sample_in_real_foundation(guest_image, cpu):
    harness, work = guest_image
    # && proves every invocation exited successfully, even after screen scrolling.
    command = "sol-demo && sol-demo && sol-demo && echo repeated-OK\\n"
    # Each engine gets a fresh persistent disk; the fs service reads IDC slot 0.
    run = work / cpu
    run.mkdir()
    (run / "disk.img").write_bytes((work / "foundation.img").read_bytes())
    (run / "config.yaml").write_text(
        'disk:\n  - slot: 0\n    type: "hdd"\n    image: "disk.img"\n    read_only: false\n', encoding="utf-8")
    result = subprocess.run([harness, str(work / "foundation.wax"), "42", "--cpu", cpu,
                             "--frames", "4200", "--dump-pcg", "--type", f"40={command}"],
                            cwd=run, capture_output=True, text=True, encoding="utf-8", errors="replace",
                            timeout=300)
    assert result.returncode == 0, result.stdout + result.stderr
    screen = result.stdout.split("PCG SCREEN BEGIN\n")[1].split("PCG SCREEN END")[0]
    assert "fault" not in screen, screen
    assert "read/close: WAX magic OK\nwrite/read/close: 5001 bytes OK" in screen, screen
    assert "read-only stream: access error OK" in screen, screen
    assert "directory: 65 entries, UTF-8, rewind, END OK\ncleanup: file and directory removed" in screen, screen
    assert "hello from sol child\nfrom-parent\nchild: exit 17 (wait closed handle)" in screen, screen
    assert "stream write OK\nsample: OK\nrepeated-OK\n/>" in screen, screen
    sys.path.insert(0, str(ROOT / "tools/fs2"))
    import fs2
    disk = bytearray((run / "disk.img").read_bytes())
    # The machine stopped while mounted: inspect DIRTY media without clearing it.
    assert fs2.check_image(disk, allow_dirty=True) == []
    volume = fs2.Volume(disk, allow_dirty=True)
    # Raw host inspection confirms cleanup and that the fixture was retained.
    assert volume.read(volume.resolve("/data/sol-fixtures/日本語.txt")) == b"UTF-8 fixture\n"
    for path in ("/sol-result.bin", "/sol-empty"):
        with pytest.raises(fs2.Fs2Error, match="NOT_FOUND"):
            volume.resolve(path)


def test_static_libfnd_preserves_sol_frame_in_user_mode(tmp_path):
    """Static link path: actual C libfnd, syscall harness with no startup roles."""
    harness = _testbench()
    pytest.importorskip("llvmlite")
    try:
        clang = find_clang(os.environ.get("CLANG"))
    except DriverError as exc:
        pytest.skip(str(exc))
    from asm import Assembler
    from wax import parse, pack
    source = tmp_path / "static.sol"
    source.write_text('!abi_compat "Foundation"\n'
                      f'!include "{LIB.as_posix()}"\n'
                      'fn probe (n) : local saved\n'
                      'n >saved\n'
                      'fnd_init 0 neq if -1 ret end\n'
                      'fnd_argc 0 neq if -2 ret end\n'
                      '0 "x" 1 0 fnd_write FND_E_BAD_HANDLE neq if -3 ret end\n'
                      'saved ret ;\n42 probe fnd_exit\n', encoding="utf-8")
    image = parse(build(source, tmp_path / "static.wax", clang=clang).read_bytes())
    loader = Assembler(strip=True, sparse=True)
    loader.assemble(_harness_asm(image))
    blocks = list(loader.blocks) + [(b["load_address"], b["data"]) for b in image["blocks"]]
    program = tmp_path / "harness.wax"
    program.write_bytes(pack(0, 0, blocks))
    (tmp_path / "config.yaml").write_text("", encoding="utf-8")
    result = subprocess.run([harness, str(program), "42", "--cpu", "pipeline"], cwd=tmp_path,
                            capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60)
    assert result.returncode == 0, result.stdout + result.stderr
