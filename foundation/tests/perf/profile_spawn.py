"""Build/run an isolated, instrumented Foundation echo spawn (Issue #106).

python foundation/tests/perf/profile_spawn.py target/spawn-profile --cpu cached
Set FND_MM=slot to force slots. Production sources are never rewritten.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
FOUNDATION = ROOT / 'foundation'


def replace(text, old, new):
    if text.count(old) != 1:
        raise RuntimeError(f'Expected one instrumentation anchor: {old!r}')
    return text.replace(old, new)


KERNEL_PROBES = '''
/* Diagnostic image only: selectors 106..111 record boundaries; 120 reads. */
static uint32_t prof_snap[6][6], prof_part[10];
static uint32_t prof_now(void) { return FND_REG32(0x80040448u); }
void prof_disk(uint32_t sectors) { ++prof_part[8]; prof_part[9] += sectors; }
static void prof_mark(uint32_t n) {
    if (n == 0) {
        for (uint32_t i = 0; i < 10; ++i) prof_part[i] = 0;
        prof_part[5] = (uint32_t)mm_vm;
    }
    sched_cpu_snapshot(prof_snap[n]);
    prof_snap[n][4] = fnd_ticks();
    prof_snap[n][5] = 0;
    for (uint32_t i = 0; i < FND_MAX_PROCS; ++i)
        if (procs[i].state != FND_PROC_FREE && procs[i].pid == 3)
            prof_snap[n][5] = procs[i].cpu_cycles;
}
'''


def kernel_source(text):
    text = replace(text, 'static uint32_t boot_drive;', 'static uint32_t boot_drive;\n' + KERNEL_PROBES)
    text = replace(text, '    sched_kill_proc(proc);', '    uint32_t prof_exit = prof_now();\n    sched_kill_proc(proc);')
    text = replace(text, '    ipc_proc_exited(proc);', '    prof_part[4] += prof_now() - prof_exit;\n    ipc_proc_exited(proc);')
    text = replace(text, '    if ((error = wax_plan(', '    uint32_t prof_plan = prof_now();\n    if ((error = wax_plan(')
    text = replace(text, '    if (!mm_vm && launch.region_count', '    prof_part[1] += prof_now() - prof_plan;\n    if (!mm_vm && launch.region_count')
    text = replace(text, '    if (mm_vm) {\n        /* The whole address space', '    uint32_t prof_map = prof_now();\n    if (mm_vm) {\n        /* The whole address space')
    text = replace(text, '    /* Nothing has changed until here', '    prof_part[2] += prof_now() - prof_map;\n    uint32_t prof_load = prof_now();\n    /* Nothing has changed until here')
    text = replace(text, '    for (uint32_t i = 0; i < launch.region_count; ++i) {\n        proc->regions', '    prof_part[3] += prof_now() - prof_load;\n    for (uint32_t i = 0; i < launch.region_count; ++i) {\n        proc->regions')
    text = replace(text, '    /* The WALs follow the image', '    uint32_t prof_copy = prof_now();\n    prof_part[6] = sp.image_len;\n    /* The WALs follow the image')
    text = replace(text, '            libs[i] = (struct fnd_image)', '            prof_part[7] += len;\n            libs[i] = (struct fnd_image)')
    text = replace(text, '    int32_t pid = spawn_image(name, sp.priority', '    prof_part[0] += prof_now() - prof_copy;\n    int32_t pid = spawn_image(name, sp.priority')
    text = replace(text, '    uint32_t live = 0, used = 0, records = 0;', '''    if (sel >= 106 && sel <= 111) {
        prof_mark(sel - 106); return 0;
    }
    if (sel == 120) {
        uint32_t size = sizeof prof_snap + sizeof prof_part;
        if (cap < size || (buf & 3u)) return FND_E_BAD_ARG;
        if (!proc_user_range_ok(caller, buf, size, CPT32_PERM_W)) return FND_E_ACCESS;
        mm_copy_to_user(caller, buf, prof_snap, sizeof prof_snap);
        mm_copy_to_user(caller, buf + sizeof prof_snap, prof_part, sizeof prof_part);
        return (int32_t)size;
    }
    uint32_t live = 0, used = 0, records = 0;''')
    return text


def library_source(text):
    text = replace(text, '    uint8_t *image;\n    uint32_t size;\n    fnd_status status = read_image(path, &image, &size);',
                   '    fnd_sys_info(106, 0, 0);\n    uint8_t *image;\n    uint32_t size;\n    fnd_status status = read_image(path, &image, &size);\n    fnd_sys_info(107, 0, 0);')
    text = replace(text, '    if (status == FND_OK)\n        status = spawn(image', '    fnd_sys_info(108, 0, 0);\n    if (status == FND_OK)\n        status = spawn(image')
    text = replace(text, '    for (uint32_t i = 0; i < deps->head.count; ++i)\n        fnd_mem_free', '    fnd_sys_info(109, 0, 0);\n    for (uint32_t i = 0; i < deps->head.count; ++i)\n        fnd_mem_free')
    text = replace(text, '    fnd_mem_free(image);\n    return status;\n}', '    fnd_mem_free(image);\n    fnd_sys_info(110, 0, 0);\n    return status;\n}')
    return text


def shell_source(text):
    text = replace(text, '    child_proc = 0;\n    if (break_hit)', '''    fnd_sys_info(111, 0, 0);
    uint32_t p[46];
    if (fnd_sys_info(120, p, sizeof p) != sizeof p) return FND_E_FORMAT;
    const char *labels[5] = {"image", "deps", "spawn", "free", "wait"};
    for (uint32_t i = 0; i < 5; ++i) {
        out(labels[i]);
        /* elapsed, kernel, idle, ticks, fs user cycles */
        uint32_t fields[5] = {0, 1, 2, 4, 5};
        for (uint32_t j = 0; j < 5; ++j) {
            uint32_t k = fields[j]; out(" ");
            out_dec((int32_t)(p[(i+1)*6+k] - p[i*6+k]), 0);
        }
        out("\\n");
    }
    out("parts");
    for (uint32_t i = 36; i < 44; ++i) { out(" "); out_dec((int32_t)p[i], 0); }
    out("\\n");
    out("io "); out_dec((int32_t)p[44], 0); out(" "); out_dec((int32_t)p[45], 0); out("\\n");
    if (status != FND_OK || code != 0) out("PROFILE CHILD FAILED\\n");
    child_proc = 0;
    if (break_hit)''')
    return text


def block_source(text):
    return replace(text, 'static void start(const struct blk_req *r) {',
                   'extern void prof_disk(uint32_t sectors);\n'
                   'static void start(const struct blk_req *r) {\n    prof_disk(r->count);')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('out', type=Path)
    parser.add_argument('--cpu', choices=['cached', 'pipeline'], default='cached')
    parser.add_argument('--clang')
    parser.add_argument('--reuse', action='store_true', help='run an existing diagnostic build')
    parser.add_argument('--fast-disk', action='store_true', help='diagnostic disk: zero latency, 1 GB/s')
    parser.add_argument('--testbench', type=Path,
                        default=Path(os.environ.get('SRC32_TESTBENCH', str(ROOT / 'target/release' /
                                     ('src32_testbench.exe' if os.name == 'nt' else 'src32_testbench')))))
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    temp = out / 'tmp'
    temp.mkdir(exist_ok=True)
    for key in ('TEMP', 'TMP', 'TMPDIR'):
        os.environ[key] = str(temp)
    # Override driver input paths only; the build uses the normal optimization,
    # module order, filesystem, commands and WAL exports.
    spec = importlib.util.spec_from_file_location('profile_boot_build', FOUNDATION / 'boot/build.py')
    build = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(build)
    transforms = {
        FOUNDATION / 'kernel/proc.c': kernel_source,
        FOUNDATION / 'lib/fnd/proc.c': library_source,
        FOUNDATION / 'shell/shell.c': shell_source,
        FOUNDATION / 'kernel/blk.c': block_source,
    }
    copies = {}
    if not args.reuse:
        for path, transform in transforms.items():
            dest = out / (path.parent.name + '_' + path.name)
            dest.write_text(transform(path.read_text(encoding='utf-8')), encoding='utf-8')
            copies[path] = dest
    original = build.driver

    def driver(sources, output, options, *extra):
        # init's static libfnd stays untouched; only the WAL gets user probes.
        sources = [copies.get(p, p) if p != build.LIBFND_PROC or output.suffix == '.wal' else p
                   for p in sources]
        return original(sources, output, options, '-I', str(FOUNDATION / 'kernel'),
                        '-I', str(FOUNDATION / 'bin'), *extra)

    build.driver = driver
    command = [str(out), '-j', '4', '--stop-ticks', '800']
    if args.clang:
        command += ['--clang', args.clang]
    if not args.reuse:
        build.main(command)
    run = out / ('run-' + args.cpu + ('-fast' if args.fast_disk else ''))
    run.mkdir(exist_ok=True)
    (run / 'disk.img').write_bytes((out / 'foundation.img').read_bytes())
    config = ('memory:\n  ram_mib: 16\n  vram_mib: 4\ndisk:\n'
              '  - slot: 0\n    type: hdd\n    image: disk.img\n')
    if args.fast_disk:
        config += '    access_latency_ms: 0\n    transfer_rate_bytes_per_second: 1000000000\n'
    (run / 'config.yaml').write_text(config, encoding='utf-8')
    result = subprocess.run([str(args.testbench.resolve()), str(out / 'foundation.wax'), '42',
                             '--cpu', args.cpu, '--frames', '660', '--dump-pcg',
                             '--type', r'180=echo\necho\n'], cwd=run, capture_output=True,
                            text=True, encoding='utf-8', errors='replace', timeout=600)
    (run / 'result.txt').write_text(result.stdout + result.stderr, encoding='utf-8')
    print(result.stdout + result.stderr)
    if result.returncode:
        raise SystemExit(result.returncode)
    if 'parts ' not in result.stdout or 'init:' in result.stdout or 'PROFILE CHILD FAILED' in result.stdout:
        raise RuntimeError('Profiling command did not complete cleanly; inspect result.txt')
    samples = []
    for line in result.stdout.splitlines():
        fields = line.split()
        if fields and fields[0] == 'image':
            samples.append({})
        if fields and fields[0] in ('image', 'deps', 'spawn', 'free', 'wait', 'parts', 'io'):
            samples[-1][fields[0]] = list(map(int, fields[1:]))
    if len(samples) != 2 or any(set(s) != {'image', 'deps', 'spawn', 'free', 'wait', 'parts', 'io'} for s in samples):
        raise RuntimeError('Expected two complete echo profiles; inspect result.txt')
    for sample in samples:
        if (any(len(sample[s]) != 5 for s in ('image', 'deps', 'spawn', 'free', 'wait')) or
                len(sample['parts']) != 8 or len(sample['io']) != 2):
            raise RuntimeError('Unexpected profile record size; inspect result.txt')
        sample['elapsed'] = sum(sample[s][0] for s in ('image', 'deps', 'spawn', 'free', 'wait'))
    (run / 'samples.json').write_text(json.dumps(samples, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
