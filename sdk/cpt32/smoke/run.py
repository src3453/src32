#!/usr/bin/env python3
"""Build and execute C guest library smokes without inheriting host BIOS/disks."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import sysconfig
import tempfile

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent


def run(*command, cwd=ROOT):
    print('+', ' '.join(map(str, command)), flush=True)
    subprocess.run(list(map(str, command)), cwd=cwd, stdin=subprocess.DEVNULL, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--clang', default=os.environ.get('CLANG'))
    parser.add_argument('--testbench', type=Path)
    parser.add_argument('--optimization', choices=['0', '1', '2'], default='0')
    parser.add_argument('--cpu', action='append', choices=['scalar', 'cached', 'cached-relaxed', 'pipeline'])
    args = parser.parse_args()
    clang = args.clang or shutil.which('clang')
    if not clang and Path('C:/Program Files/LLVM/bin/clang.exe').is_file():
        clang = 'C:/Program Files/LLVM/bin/clang.exe'
    if not clang:
        parser.error('Clang is required; set --clang or CLANG')
    suffix = '.exe' if os.name == 'nt' else ''
    ninja = shutil.which('ninja') or str(Path(sysconfig.get_path('scripts')) / ('ninja' + suffix))
    if not Path(ninja).is_file():
        parser.error('Install Ninja with: python -m pip install ninja')
    build = ROOT / 'target' / 'cpt32-c-smoke'
    run('cmake', '-S', HERE, '-B', build, '-G', 'Ninja',
        '-DSRC32_PYTHON_EXECUTABLE=' + sys.executable,
        '-DSRC32_CLANG=' + clang, '-DCMAKE_MAKE_PROGRAM=' + ninja,
        '-DCPT32_SMOKE_OPTIMIZATION=' + args.optimization)
    run('cmake', '--build', build)
    harness = args.testbench
    if harness is None:
        run('cargo', 'build', '--bin', 'src32_testbench')
        target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target'))
        if not target.is_absolute():
            target = ROOT / target
        harness = target / 'debug' / ('src32_testbench' + suffix)
    harness = harness.resolve()
    for cpu in args.cpu or ['scalar', 'pipeline']:
        for name in ['display', 'irqc', 'sgub', 'wax', 'storage']:
            # The storage guest writes only a disposable disk, recreated per run.
            with tempfile.TemporaryDirectory(prefix='cpt32-c-') as temporary:
                work = Path(temporary)
                if name == 'storage':
                    disk = work / 'disk0.img'
                    run(sys.executable, ROOT / 'tools/dsfs/dsfs.py', 'mkfs', disk, '--sectors', '64')
                    (work / 'config.yaml').write_text(
                        'disk:\n  - slot: 0\n    type: fdd\n    image: disk0.img\n'
                        '    read_only: false\n    transfer_rate_bytes_per_second: 48000000\n'
                        '    access_latency_ms: 0\n', encoding='utf-8')
                run(harness, build / (name + '.wax'), '42', '--cpu', cpu, '--frames', '32', cwd=work)
                if name == 'storage':
                    run(sys.executable, ROOT / 'tools/dsfs/dsfs.py', 'verify', disk)
    print('CPT32 C library guest smokes passed.')


if __name__ == '__main__':
    main()
