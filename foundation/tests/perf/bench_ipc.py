"""Measure real Foundation U-mode IPC in guest 48 MHz clock ticks.

python foundation/tests/perf/bench_ipc.py target/ipc-bench --mm vm slot
No production sources are instrumented. See README.md for interpretation.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
KERNEL = ('sched', 'proc', 'mm', 'trap', 'ipc', 'fb', 'sgu', 'sgc', 'pec', 'blk', 'wax')
CASES = [('time_get', 0, 0)] + [
    (mode, size, offset) for mode in ('reply+recv', 'reply_recv')
    for size, offset in ((0, 0), (32, 0), (256, 0), (4096, 0), (4096, 1))]


def run(command, cwd, timeout):
    result = subprocess.run(list(map(str, command)), cwd=cwd, capture_output=True,
                            text=True, encoding='utf-8', errors='replace', timeout=timeout)
    if result.returncode:
        raise RuntimeError(f'Command failed: {command}\n{result.stdout}{result.stderr}')
    return result.stdout + result.stderr


def parse_samples(output):
    rows = re.findall(r'^IPC (\d+) +(\d+) +(\d+) +(\d+) *$', output, re.MULTILINE)
    if [int(row[0]) for row in rows] != list(range(len(CASES))):
        raise ValueError('Missing, duplicate, or out-of-order IPC samples')
    samples = [dict(zip(('elapsed', 'kernel', 'idle'), map(int, row[1:]))) for row in rows]
    for row in samples:
        if not 0 < row['elapsed'] < 2**31 or row['kernel'] + row['idle'] > row['elapsed']:
            raise ValueError(f'Invalid accounting delta: {row}')
    if 'PASS:' not in output:
        raise ValueError('Guest did not complete successfully')
    return samples


def positive(value):
    n = int(value)
    if n <= 0:
        raise argparse.ArgumentTypeError('must be positive')
    return n


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('out', type=Path)
    parser.add_argument('--mm', nargs='+', choices=['vm', 'slot'], default=['vm', 'slot'])
    parser.add_argument('--cpu', choices=['scalar', 'cached', 'cached-relaxed', 'pipeline'], default='cached')
    parser.add_argument('--rounds', type=positive, default=64)
    parser.add_argument('--repeats', type=positive, default=3)
    parser.add_argument('--priority', choices=['service', 'equal'], default='service')
    parser.add_argument('--dma-min', type=int, default=256,
                        help='minimum physical copy size for DMA; 0 disables DMA')
    parser.add_argument('--clang', default=os.environ.get('CLANG') or shutil.which('clang') or
                        'C:/Program Files/AMD/ROCm/6.4/bin/clang.exe')
    parser.add_argument('--testbench', type=Path, default=Path(os.environ.get('SRC32_TESTBENCH',
                        str(ROOT / 'target/release' / ('src32_testbench.exe' if os.name == 'nt' else 'src32_testbench')))))
    args = parser.parse_args()
    if args.dma_min < 0:
        parser.error('--dma-min must be non-negative')
    # Each delta must stay well below the 32-bit accounting clock wrap.
    if args.rounds > 512:
        parser.error('--rounds must be <= 512; increase --repeats for more samples')
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    temp = out / 'tmp'
    temp.mkdir(exist_ok=True)
    for key in ('TEMP', 'TMP', 'TMPDIR'):
        os.environ[key] = str(temp)
    harness = args.testbench.resolve()
    clang = shutil.which(args.clang) or args.clang
    metadata = dict(cpu=args.cpu, rounds=args.rounds, repeats=args.repeats, priority=args.priority,
                    copy_dma_min=args.dma_min,
                    clock_hz=48000000, optimization='-O1', warmup=8,
                    git=run(['git', 'rev-parse', 'HEAD'], ROOT, 10).strip(),
                    dirty=bool(run(['git', 'status', '--porcelain'], ROOT, 10).strip()),
                    clang=run([clang, '--version'], ROOT, 10).splitlines()[0],
                    testbench_sha256=hashlib.sha256(harness.read_bytes()).hexdigest(), images={})
    results = []
    for mm in dict.fromkeys(args.mm):
        work = out / mm
        work.mkdir(exist_ok=True)
        (work / 'config.yaml').write_text('memory:\n  ram_mib: 16\n  vram_mib: 4\n', encoding='utf-8')
        image = work / 'ipc.wax'
        sources = [HERE / 'ipc_user.c', HERE / 'ipc_main.c']
        sources += [ROOT / 'foundation/kernel' / (name + '.c') for name in KERNEL]
        command = [sys.executable, ROOT / 'tools/llvm-src32/driver.py', '--clang', clang, *sources,
                   '-I', ROOT / 'sdk/cpt32/include', '-I', ROOT / 'foundation/include',
                   '-I', ROOT / 'foundation/kernel', '--origin', '0x10000', '--stack-top', '0x100000',
                   '-O1', '-D', f'FND_COPY_DMA_MIN={args.dma_min}', '-D', f'BENCH_ROUNDS={args.rounds}', '-D',
                   'BENCH_SERVER_PRIORITY=' + ('1' if args.priority == 'service' else '2'), '-o', image]
        if mm == 'slot':
            command += ['-D', 'FND_MM_NOMMU_ONLY']
        (work / 'build.txt').write_text(run(command, work, 300), encoding='utf-8')
        metadata['images'][mm] = hashlib.sha256(image.read_bytes()).hexdigest()
        repeats = []
        for repeat in range(args.repeats):
            output = run([harness, image, '42', '--cpu', args.cpu, '--frames', '1500', '--dump-pcg'], work, 600)
            (work / f'run-{repeat}.txt').write_text(output, encoding='utf-8')
            repeats.append(parse_samples(output))
        for index, (mode, size, offset) in enumerate(CASES):
            samples = [repeat[index] for repeat in repeats]
            elapsed = [s['elapsed'] / args.rounds for s in samples]
            median = statistics.median(elapsed)
            row = dict(mm=mm, mode=mode, bytes=size, offset=offset, samples=samples,
                       ticks_per_op_median=median, ticks_per_op_min=min(elapsed),
                       ticks_per_op_max=max(elapsed), us_per_op=median / 48,
                       ops_per_second=48000000 / median,
                       kernel_ticks_per_op=statistics.median(s['kernel'] / args.rounds for s in samples),
                       idle_ticks_per_op=statistics.median(s['idle'] / args.rounds for s in samples))
            results.append(row)
            print(f'{mm:4} {mode:10} {size:4}+{offset}  {median:10.1f} ticks/op  '
                  f'{row["us_per_op"]:8.2f} us/op  kernel={row["kernel_ticks_per_op"]:.1f}')
    (out / 'results.json').write_text(json.dumps(dict(metadata=metadata, results=results), indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
