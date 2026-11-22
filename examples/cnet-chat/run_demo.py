"""Two Foundation emulators talking over CNET (doc/spec_NET.md).

Builds Foundation, the CNET card and the chat program, then starts two
emulators on one UDP segment on this machine:

    A  node 0001.0010   type:  chat 20
    B  node 0001.0020   type:  chat 10

Each window then sends every typed line to the other. The shell's own
`cnet ping 20` / `cnet send 20 77 hello` work as well.

    python examples/cnet-chat/run_demo.py [--clang PATH]
    python examples/cnet-chat/run_demo.py --headless   # scripted, two testbenches

--headless runs two src32_testbench processes at once, types a short
conversation into both and checks that each screen shows the other's lines
(exit status 0 on success). It needs a built src32_testbench
(cargo build --release --bin src32_testbench, or SRC32_TESTBENCH).
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
OUT = ROOT / "target/cnet-demo"
NET = 1
NODES = {"A": 0x0010, "B": 0x0020}
SEGMENT_ID = 0x0C4A
CARD_WASM = ROOT / "examples/wasm-cnet/target/wasm32-unknown-unknown/release/cpt32_wasm_cnet_card.wasm"
sys.path.insert(0, str(ROOT / "tools/fs2"))
import fs2  # noqa: E402

# Headless script: frame -> text for each side (60 frames per second).
SCRIPT = {
    "A": [(150, "chat 20\\n"), (300, "hello from A\\n"), (520, "fine, bye\\n")],
    "B": [(150, "chat 10\\n"), (420, "hi A, this is B\\n")],
}
HEADLESS_FRAMES = 660


def run(command, **kwargs):
    print("+", " ".join(map(str, command)), flush=True)
    subprocess.run(list(map(str, command)), check=True, cwd=ROOT, **kwargs)


def build(args):
    foundation = OUT / "foundation"
    command = [sys.executable, ROOT / "foundation/boot/build.py", foundation]
    if args.clang:
        command += ["--clang", args.clang]
    run(command)
    run(["cargo", "build", "--manifest-path", ROOT / "examples/wasm-cnet/Cargo.toml", "--release",
         "--target", "wasm32-unknown-unknown"])
    card = OUT / "cnet.cpt32card"
    with zipfile.ZipFile(card, "w", compression=zipfile.ZIP_DEFLATED) as package:
        package.writestr("manifest.yaml", "abi_version: 1\nmodule: card.wasm\ncapabilities: [network]\n")
        package.write(CARD_WASM, "card.wasm")
    chat = OUT / "chat.wax"
    command = [sys.executable, ROOT / "tools/llvm-src32/driver.py", HERE / "chat.c",
               "-I", ROOT / "sdk/cpt32/include", "-I", ROOT / "foundation/include", "-O1",
               "--wax3", "--page-align", "--image-id", "example.cnet_chat",
               "--import-wal", foundation / "root/lib/fnd.1.wal",
               "--import-wal", foundation / "root/lib/cnet.1.wal",
               "--pie", "--user-stack-size", "0x4000", "-o", chat]
    if args.clang:
        command[2:2] = ["--clang", args.clang]
    run(command)
    volume = fs2.Volume(bytearray((foundation / "foundation.img").read_bytes()))
    volume.put("/bin/chat.wax", chat.read_bytes(), mode=0x3D)
    return foundation, card, bytes(volume.img)


def free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def prepare(name, disk, card, port, peer_port, gui):
    """Writes OUT/<name>/ with its own disk copy and config.yaml."""
    directory = OUT / name
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "disk.img").write_bytes(disk)
    shutil.copyfile(card, directory / "cnet.cpt32card")
    config = "memory:\n  ram_mib: 16\n  vram_mib: 4\n"
    if gui:
        config = "cpu:\n  microarchitecture: cached-relaxed\n" + config
        config += f"pcg:\n  font: {json.dumps(str(ROOT / 'assets/UNSCII.F08'))}\n"
    config += ('disk:\n  - slot: 0\n    type: "hdd"\n    image: "disk.img"\n    read_only: false\n'
               "network_segment:\n"
               f"  - name: lan\n    network: {NET}\n"
               f'    udp:\n      bind: "127.0.0.1:{port}"\n      peers: ["127.0.0.1:{peer_port}"]\n'
               f"      segment_id: {SEGMENT_ID}\n"
               "expansion_card:\n"
               "  - slot: 0\n    package: cnet.cpt32card\n"
               f"    network: {{ segment: lan, node: {NODES[name]} }}\n")
    (directory / "config.yaml").write_text(config, encoding="utf-8")
    return directory


def testbench():
    if os.environ.get("SRC32_TESTBENCH"):
        return Path(os.environ["SRC32_TESTBENCH"])
    suffix = ".exe" if os.name == "nt" else ""
    for profile in ("release", "debug"):
        candidate = ROOT / "target" / profile / f"src32_testbench{suffix}"
        if candidate.is_file():
            return candidate
    raise SystemExit("build src32_testbench first (cargo build --release --bin src32_testbench)")


def screen(stdout):
    lines = stdout.splitlines()
    rows = lines[lines.index("PCG SCREEN BEGIN") + 1:lines.index("PCG SCREEN END")]
    while rows and not rows[-1].strip():
        rows.pop()
    return rows


def headless(image, directories):
    harness = testbench()
    processes = {}
    for name, directory in directories.items():
        command = [harness, image, "0", "--allow-running", "--cpu", "cached",
                   "--frames", str(HEADLESS_FRAMES), "--dump-pcg"]
        for frame, text in SCRIPT[name]:
            command += ["--type", f"{frame}={text}"]
        processes[name] = subprocess.Popen(list(map(str, command)), cwd=directory, stdout=subprocess.PIPE,
                                           stderr=subprocess.PIPE, text=True, encoding="utf-8",
                                           errors="replace")
    screens = {}
    for name, process in processes.items():
        stdout, stderr = process.communicate(timeout=600)
        if process.returncode != 0:
            raise SystemExit(f"testbench {name} failed:\n{stdout}\n{stderr}")
        screens[name] = screen(stdout)
        print(f"===== {name} (node {NET:04X}.{NODES[name]:04X}) =====")
        print("\n".join(screens[name]))
    expected = {"A": ["<0001.0020> hi A, this is B"],
                "B": ["<0001.0010> hello from A", "<0001.0010> fine, bye"]}
    missing = [(name, line) for name, lines in expected.items() for line in lines
               if not any(line in row for row in screens[name])]
    if missing:
        print("missing:", missing)
        return 1
    print("both sides received the other's messages")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--clang", help="Clang executable passed to the SRC32 compiler")
    parser.add_argument("--headless", action="store_true", help="scripted run on two testbenches")
    args = parser.parse_args()
    foundation, card, disk = build(args)
    ports = {"A": free_port(), "B": free_port()}
    directories = {name: prepare(name, disk, card, ports[name], ports["B" if name == "A" else "A"],
                                 not args.headless) for name in NODES}
    image = foundation / "foundation.wax"
    if args.headless:
        return headless(image, directories)
    run(["cargo", "build", "--release", "--bin", "cpt32"])
    executable = ROOT / "target/release" / ("cpt32.exe" if os.name == "nt" else "cpt32")
    print("A: type `chat 20`   B: type `chat 10`", flush=True)
    processes = [subprocess.Popen([str(executable), str(image)], cwd=directory)
                 for directory in directories.values()]
    return max(process.wait() for process in processes)


if __name__ == "__main__":
    raise SystemExit(main())
