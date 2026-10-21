"""Behavioral regressions. Requires llvmlite, Clang and native src32_testbench.

Run: python -m unittest discover -s tools/llvm-src32 -p 'test_*.py' -v
CLANG and SRC32_TESTBENCH override executable discovery. No BIOS or host disk
configuration is inherited: execution uses an isolated working directory.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
LAYOUT = 'target datalayout = "E-m:m-p:32:32-i8:8:32-i16:16:32-i64:64-n32-S64"\n'
TRIPLE = 'target triple = "mips-unknown-none"\n'


class ToolchainTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.clang = os.environ.get('CLANG') or shutil.which('clang')
        if not cls.clang:
            candidate = Path('C:/Program Files/AMD/ROCm/6.4/bin/clang.exe')
            if candidate.is_file():
                cls.clang = str(candidate)
        if not cls.clang:
            raise RuntimeError('Install Clang or set CLANG to its executable')
        suffix = '.exe' if os.name == 'nt' else ''
        cls.harness = Path(os.environ.get(
            'SRC32_TESTBENCH', str(ROOT / 'target/debug' / ('src32_testbench' + suffix))))
        if not cls.harness.is_file():
            raise RuntimeError('Build cargo build --bin src32_testbench or set SRC32_TESTBENCH')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='llvm-src32-')
        self.addCleanup(self.temp.cleanup)
        self.work = Path(self.temp.name)

    def driver(self, *args, success=True):
        result = subprocess.run(
            [sys.executable, str(HERE / 'driver.py'), '--clang', self.clang,
             *map(str, args)], cwd=self.work, capture_output=True, text=True,
            encoding='utf-8', errors='replace', timeout=120)
        output = result.stdout + result.stderr
        if success:
            self.assertEqual(result.returncode, 0, output)
        else:
            self.assertNotEqual(result.returncode, 0, output)
            self.assertNotIn('Traceback (most recent call last)', output)
        return result

    def execute(self, image, expected, cpu='scalar'):
        result = subprocess.run(
            [str(self.harness), str(image), str(expected), '--cpu', cpu],
            cwd=self.work, capture_output=True, text=True, encoding='utf-8',
            errors='replace', timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('PASS:', result.stdout)

    def ir(self, text, name='case.ll'):
        path = self.work / name
        path.write_text(LAYOUT + TRIPLE + text, encoding='utf-8')
        return path

    def test_multifile_c_project(self):
        sources = [HERE / 'examples/conformance' / name
                   for name in ('main.c', 'operations.c', 'integers.c', 'wide.c')]
        for opt in ('-O0', '-O1', '-O2'):
            with self.subTest(optimization=opt):
                image = self.work / (opt[1:] + '.wax')
                self.driver(*sources, opt, '-o', image)
                for cpu in ('scalar', 'cached', 'cached-relaxed', 'pipeline'):
                    with self.subTest(cpu=cpu):
                        self.execute(image, 42, cpu)

    def test_unsigned_multiply_overflow_and_three_way_compare(self):
        # Keep operands dynamic so Clang cannot constant-fold the boundaries.
        source = self.work / 'overflow.c'
        source.write_text('''
static volatile unsigned pairs[][2] = {
    {0, 0xffffffffu}, {0xffffffffu, 0}, {0xffffffffu, 1},
    {0x80000000u, 2}, {0x10000u, 0x10000u},
    {0x7fffffffu, 2}, {0xffffffffu, 0xffffffffu}};
static int cmp(int a, int b) { return (a > b) - (a < b); }
int main(void) {
    for (unsigned i=0; i<7; ++i) {
        unsigned a=pairs[i][0], b=pairs[i][1], product;
        unsigned long long full=(unsigned long long)a*b;
        int overflow=__builtin_mul_overflow(a,b,&product);
        if (product != (unsigned)full || overflow != (full > 0xffffffffu)) return 100+i;
    }
    if(cmp(-2147483647-1,2147483647)!=-1 || cmp(1,-1)!=1 || cmp(-1,-1)!=0) return 110;
    return 42;
}
''', encoding='utf-8')
        for opt in ('-O0', '-O1', '-O2'):
            with self.subTest(optimization=opt):
                image = self.work / 'overflow.wax'
                self.driver(source, opt, '-o', image)
                self.execute(image, 42)
                self.execute(image, 42, 'pipeline')

    def test_constant_pointer_arithmetic(self):
        source = self.ir('''
@data = global [4 x i8] c"abc\\00"
@address = global i32 add (i32 ptrtoint (ptr @data to i32), i32 2)
@distance = global i32 sub (i32 ptrtoint (ptr @data to i32), i32 ptrtoint (ptr getelementptr ([4 x i8], ptr @data, i32 0, i32 2) to i32))
define i32 @main() {
  %p = load i32, ptr @address
  %ptr = inttoptr i32 %p to ptr
  %byte = load i8, ptr %ptr
  %d = load i32, ptr @distance
  %ok1 = icmp eq i8 %byte, 99
  %ok2 = icmp eq i32 %d, -2
  %ok = and i1 %ok1, %ok2
  %result = select i1 %ok, i32 42, i32 100
  ret i32 %result
}
''')
        image = self.work / 'constant.wax'
        self.driver(source, '-o', image)
        self.execute(image, 42)

    def test_three_way_compare_intrinsics(self):
        functions, checks = [], []
        for index, (a, b) in enumerate(((0, 0), (0, 1), (1, 0),
                                      (0x80000000, 0x7fffffff), (0xffffffff, 0))):
            for kind in ('scmp', 'ucmp'):
                signed = lambda n: n - 0x100000000 if n & 0x80000000 else n
                x, y = (signed(a), signed(b)) if kind == 'scmp' else (a, b)
                expected = (x > y) - (x < y)
                name = f'{kind}_{index}'
                functions.append(f'''define i32 @{name}(i32 %a, i32 %b) {{
  %r = call i32 @llvm.{kind}.i32.i32(i32 %a, i32 %b)
  ret i32 %r
}}''')
                checks.append(f'''  %v{len(checks)} = call i32 @{name}(i32 {a}, i32 {b})
  %ok{len(checks)} = icmp eq i32 %v{len(checks)}, {expected}''')
        main = '\n'.join(checks)
        main += '\n  %sum0 = and i1 %ok0, %ok1\n'
        for i in range(2, len(checks)):
            main += f'  %sum{i-1} = and i1 %sum{i-2}, %ok{i}\n'
        main += f'  %result = select i1 %sum{len(checks)-2}, i32 42, i32 100\n  ret i32 %result\n'
        source = self.ir('declare i32 @llvm.scmp.i32.i32(i32, i32)\n'
                         'declare i32 @llvm.ucmp.i32.i32(i32, i32)\n' +
                         '\n'.join(functions) + '\ndefine i32 @main() {\n' + main + '}\n')
        image = self.work / 'compare.wax'
        self.driver(source, '-o', image)
        self.execute(image, 42)
        self.execute(image, 42, 'pipeline')

    def test_unsigned_saturating_subtraction(self):
        # Direct IR prevents Clang folding constants or replacing the intrinsic.
        # Include non-C widths, the unsigned sign boundary, and both i64 words.
        declarations, functions, checks = [], [], []
        index = 0
        for bits in (1, 7, 8, 16, 31, 32, 64):
            ty = f'i{bits}'
            maximum, sign = (1 << bits) - 1, 1 << (bits - 1)
            pairs = [(0, 0), (0, 1), (1, 0), (1, 1),
                     (maximum, maximum), (maximum, 0), (maximum, 1),
                     (0, maximum), (sign, sign - 1), (sign - 1, sign)]
            if bits == 64:
                pairs += [(0x100000000, 1), (0xffffffff, 0x100000000),
                          (0x200000000, 0x100000001),
                          (0x100000001, 0x100000002),
                          (0xffffffffffffffff, 0xffffffff)]
            declarations.append(f'declare {ty} @llvm.usub.sat.{ty}({ty}, {ty})')
            functions.append(f'''define {ty} @sub{bits}({ty} %a, {ty} %b) {{
  %r = call {ty} @llvm.usub.sat.{ty}({ty} %a, {ty} %b)
  ret {ty} %r
}}''')
            for a, b in pairs:
                checks += [f'  %r{index} = call {ty} @sub{bits}({ty} {a}, {ty} {b})',
                           f'  %ok{index} = icmp eq {ty} %r{index}, {max(a - b, 0)}',
                           f'  %all{index} = and i1 %ok{index}, ' +
                           ('true' if index == 0 else f'%all{index - 1}')]
                index += 1
        source = self.ir('\n'.join(declarations + functions) + '\n' +
                         'define i32 @main() {\n' + '\n'.join(checks) +
                         f'\n  %result = select i1 %all{index - 1}, i32 42, i32 1\n'
                         '  ret i32 %result\n}\n')
        image = self.work / 'usub-sat.wax'
        self.driver(source, '-o', image)
        for cpu in ('scalar', 'pipeline'):
            self.execute(image, 42, cpu)

    def test_softfloat_c_example(self):
        example = ROOT / 'examples/softfloat-c'
        library = example / 'vendor/softfloat/source'
        sources = sorted(library.rglob('*.c'))
        for opt in ('-O0', '-O1', '-O2'):
            with self.subTest(optimization=opt):
                image = self.work / (opt[1:] + '.wax')
                self.driver(example / 'main.c', *sources,
                            ROOT / 'sdk/cpt32/src/pcg.c',
                            '-I', example / 'port', '-I', library / 'include',
                            '-I', library / '8086', '-I', ROOT / 'sdk/cpt32/include',
                            '-DINLINE_LEVEL=0', '-DSOFTFLOAT_ROUND_ODD', opt, '-o', image)
                for cpu in ('scalar', 'pipeline'):
                    self.execute(image, 42, cpu)

    def test_cpp_example_with_c_sdk(self):
        example = ROOT / 'examples/hello-cpp'
        # Exercise each C++ extension, constructor/method mangling across TUs,
        # C SDK linkage, constexpr and PCG VRAM checks at every supported -O.
        main = self.work / 'main.cc'
        main.write_text((example / 'main.cpp').read_text(encoding='utf-8'), encoding='utf-8')
        counter = self.work / 'counter.cxx'
        counter.write_text((example / 'counter.cpp').read_text(encoding='utf-8'), encoding='utf-8')
        for opt in ('-O0', '-O1', '-O2'):
            with self.subTest(optimization=opt):
                image = self.work / (opt[1:] + '.wax')
                self.driver(main, counter, ROOT / 'sdk/cpt32/src/pcg.c',
                            ROOT / 'sdk/cpt32/src/print.c', '-I', example,
                            '-I', ROOT / 'sdk/cpt32/include', opt, '-o', image)
                for cpu in ('scalar', 'pipeline'):
                    self.execute(image, 42, cpu)
        image = self.work / 'cpp.wax'
        self.driver(example / 'main.cpp', example / 'counter.cpp',
                    ROOT / 'sdk/cpt32/src/pcg.c', ROOT / 'sdk/cpt32/src/print.c',
                    '-I', ROOT / 'sdk/cpt32/include', '-o', image)
        self.execute(image, 42)

    def test_cpp_dynamic_global_initialization_rejected(self):
        source = self.work / 'global.cpp'
        source.write_text('''
extern "C" int read_value();
int value = read_value();
extern "C" int read_value() { return 42; }
extern "C" int main() { return value; }
''', encoding='utf-8')
        result = self.driver(source, '-o', self.work / 'global.wax', success=False)
        self.assertIn('unsupported linkage appending', result.stderr)

    def test_cpt32_halt_waits_for_irq_then_resumes(self):
        source = self.work / 'halt.c'
        source.write_text('''
#include <cpt32/cpu.h>
#include <stdint.h>

#define IRQC_PENDING (*(volatile uint16_t *)0xFFFF0040u)
#define IRQC_ENABLE (*(volatile uint16_t *)0xFFFF0044u)
static volatile uint32_t wake_count;

void irqc_dispatch(void) {
    if ((IRQC_PENDING & IRQC_ENABLE & 1u) != 0u) {
        IRQC_PENDING = 1u;
        ++wake_count;
    }
}

int main(void) {
    IRQC_PENDING = 0xFFFFu;
    IRQC_ENABLE = 1u;
    cpt32_halt();
    IRQC_ENABLE = 0u;
    return wake_count != 0u ? 42 : 101;
}
''', encoding='utf-8')
        image = self.work / 'halt.wax'
        self.driver(source, '-I', ROOT / 'sdk/cpt32/include', '-o', image)
        self.execute(image, 42)
        self.execute(image, 42, 'pipeline')

    def test_user_mode_syscall_example(self):
        # Extension P: kernel.c runs user.c in user mode; user.c checks the
        # caller mode, a SYSCALL result, the context byte path and a refused
        # kernel-memory poke, then exits with 42 through SYSCALL 0.
        example = ROOT / 'examples/syscall-user-c'
        for opt in ('-O0', '-O1', '-O2'):
            with self.subTest(optimization=opt):
                image = self.work / (opt[1:] + '.wax')
                self.driver(example / 'user.c', example / 'kernel.c',
                            ROOT / 'sdk/cpt32/src/pcg.c', '-I', ROOT / 'sdk/cpt32/include',
                            opt, '-o', image)
                for cpu in ('scalar', 'cached', 'cached-relaxed', 'pipeline'):
                    with self.subTest(cpu=cpu):
                        self.execute(image, 42, cpu)

    def test_user_mode_store_to_kernel_memory_faults(self):
        # The control program exits with 42. With -DTOUCH the user code first
        # stores to kernel data; the protection fault reaches the default
        # (self-referencing) bus-error vector, which halts before the exit.
        source = self.work / 'touch.c'
        source.write_text('''#include <cpt32/cpu.h>
volatile uint32_t kernel_word;
void user_entry(void) {
#ifdef TOUCH
    kernel_word = 1;
#endif
    CPT32_SYSCALL1(0, 42);
    for (;;) {}
}
void user_end(void) {}
void syscall_dispatch(cpt32_trap_frame *frame) { cpt32_return_to_kernel(frame->r[1]); }
int main(void) {
    cpt32_mtsr(CPT32_SR_RGN_START(0), (uint32_t)(uintptr_t)user_entry);
    cpt32_mtsr(CPT32_SR_RGN_END(0), (uint32_t)(uintptr_t)user_end);
    cpt32_mtsr(CPT32_SR_RGN_FLAGS(0), CPT32_PERM_R | CPT32_PERM_X);
    cpt32_mtsr(CPT32_SR_RGN_START(1), 0x80000u);
    cpt32_mtsr(CPT32_SR_RGN_END(1), 0x90000u);
    cpt32_mtsr(CPT32_SR_RGN_FLAGS(1), CPT32_PERM_R | CPT32_PERM_W);
    if (kernel_word != 0) return 1;
    return (int)cpt32_run_user(user_entry, 0x90000u);
}
''', encoding='utf-8')
        include = ROOT / 'sdk/cpt32/include'
        control = self.work / 'control.wax'
        self.driver(source, '-I', include, '-o', control)
        touch = self.work / 'touch.wax'
        self.driver(source, '-I', include, '-DTOUCH', '-o', touch)
        for cpu in ('scalar', 'pipeline'):
            with self.subTest(cpu=cpu):
                self.execute(control, 42, cpu)
                result = subprocess.run(
                    [str(self.harness), str(touch), '42', '--cpu', cpu],
                    cwd=self.work, capture_output=True, text=True, encoding='utf-8',
                    errors='replace', timeout=120)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_protected_wax_output(self):
        sys.path.insert(0, str(ROOT / 'tools/wax'))
        from wax import parse as parse_wax, user_regions
        source = self.work / 'app.c'
        source.write_text('int counter = 5;\nint main(void) { return counter + 37; }\n',
                          encoding='utf-8')
        image = self.work / 'app.wax'
        self.driver(source, '--protected', '--origin', '0x10000', '--stack-top', '0x40000',
                    '--user-stack-size', '0x2000', '-o', image)
        parsed = parse_wax(image.read_bytes())
        self.assertEqual(parsed['protection'], (0x40000, 0x2000))
        self.assertEqual([block['perm'] for block in parsed['blocks']], [5, 3])
        self.assertEqual(user_regions(parsed)[0], (0x3E000, 0x40000, 3))
        self.assertEqual(parsed['entry'], 0x10000)
        assembly = self.work / 'app.a'
        self.driver(source, '--protected', '--origin', '0x10000', '--stack-top', '0x40000',
                    '-S', '-o', assembly)
        text = assembly.read_text(encoding='utf-8')
        self.assertIn('SYSCALL 0', text)
        self.assertNotIn('HALT', text)

    def test_data_origin_rom_layout(self):
        sys.path.insert(0, str(ROOT / 'tools/wax'))
        from wax import parse as parse_wax
        source = self.work / 'rom.c'
        source.write_text('''int counter = 5;
int zeroed;
int table[3] = {1, 2, 3};
static const char message[] = "ok";
int main(void) {
  counter += 29; zeroed += 2; table[2] += 2;
  return counter + zeroed + table[2] + (message[0] == 'o');
}
''', encoding='utf-8')
        for opt in ('-O0', '-O2'):
            with self.subTest(optimization=opt):
                image = self.work / (opt[1:] + '.wax')
                self.driver(source, opt, '--origin', '0x20000', '--data-origin', '0x80000',
                            '--stack-top', '0x100000', '-o', image)
                blocks = parse_wax(image.read_bytes())['blocks']
                # Writable data is not part of the image; only its load copy is.
                self.assertTrue(all(block['load_address'] + len(block['data']) <= 0x80000
                                    for block in blocks), blocks)
                self.execute(image, 42)
                self.execute(image, 42, 'pipeline')
        self.driver(source, '--data-origin', '0x80004', '-o', self.work / 'odd.wax', success=False)
        self.driver(source, '--data-origin', '0x80000', '--protected', '-o', self.work / 'p.wax',
                    success=False)
        overlap = self.driver(source, '--origin', '0x20000', '--data-origin', '0xFFFF8',
                              '--stack-top', '0x100000', '-o', self.work / 'overlap.wax', success=False)
        self.assertIn('writable data area', overlap.stdout + overlap.stderr)

    def test_jump_intrinsic_sets_r1_and_stack(self):
        source = self.work / 'jump.c'
        source.write_text('''#include <cpt32/cpu.h>
void landing(void) { cpt32_halt(); }
int main(void) {
  cpt32_jump((uint32_t)(uintptr_t)landing, 42u, 0x90000u);
}
''', encoding='utf-8')
        image = self.work / 'jump.wax'
        self.driver(source, '-I', ROOT / 'sdk/cpt32/include', '-o', image)
        self.execute(image, 42)
        self.execute(image, 42, 'pipeline')

    def test_extension_p_intrinsic_diagnostics(self):
        header = '#include <cpt32/cpu.h>\n'
        cases = {
            'halt.c': (header + 'int main(void) { cpt32_halt(); return 0; }\n',
                       ['--protected'], 'unavailable with --protected'),
            'handler.c': (header + 'void syscall_dispatch(cpt32_trap_frame *f) { (void)f; }\n'
                          'int main(void) { return 0; }\n', ['--protected'], 'needs supervisor mode'),
            'number.c': (header + 'int main(void) { volatile uint32_t n = 1;\n'
                         'return (int)cpt32_syscall(n, 0, 0, 0, 0, 0); }\n', [],
                         'syscall number must be an integer constant'),
            'range.c': (header + 'int main(void) { return (int)CPT32_SYSCALL0(256); }\n',
                        [], 'out of range'),
            'sr.c': (header + 'int main(void) { return (int)cpt32_mfsr(3); }\n',
                     [], 'system register 0x3 is out of range'),
        }
        for name, (text, extra, message) in cases.items():
            with self.subTest(case=name):
                source = self.work / name
                source.write_text(text, encoding='utf-8')
                result = self.driver(source, '-I', ROOT / 'sdk/cpt32/include', *extra,
                                     '-o', self.work / (name + '.wax'), success=False)
                self.assertIn(message, result.stderr)

    def test_optimized_static_helper_calls(self):
        source = self.work / 'static.c'
        source.write_text('''
#include <stdint.h>
static volatile uint32_t input = 3;
static __attribute__((noinline)) uint64_t helper(uint32_t n, uint64_t x) {
    if (n == 0) return x;
    return helper(n - 1, x + n);
}
int main(void) {
    return helper(input, UINT64_C(0x1234567800000024)) ==
           UINT64_C(0x123456780000002a) ? 42 : 1;
}
''', encoding='utf-8')
        for opt in ('-O1', '-O2'):
            with self.subTest(optimization=opt):
                temps = self.work / opt[1:]
                image = self.work / (opt[1:] + '.wax')
                self.driver(source, opt, '--save-temps', temps, '-o', image)
                # The old conformance project can inline all static helpers;
                # require an actual optimized calling convention regression.
                self.assertIn('fastcc', (temps / 'linked.ll').read_text())
                self.execute(image, 42)
                self.execute(image, 42, 'pipeline')

    def test_optimized_sgub_driver_furnace_stream(self):
        sources = [ROOT / 'sol/cpt32/sgu_music_driver.c'] + [
            ROOT / 'sdk/cpt32/src' / name
            for name in ('sgub.c', 'irqc.c', 'pcg.c')]
        for opt in ('-O0', '-O1', '-O2'):
            with self.subTest(optimization=opt):
                image = self.work / (opt[1:] + '.wax')
                self.driver(*sources, '-I', ROOT / 'sdk/cpt32/include', opt, '-o', image)
                # Validate completed guest IRQs, rather than claiming that the
                # unoptimized dense reset frame fits a real-time frame budget.
                # The bounded drain sends no additional host VSYNCs.
                # Keep Pipeline: the harness's fixed 50M startup budget is
                # insufficient for Scalar's full PCM initialization at -O0.
                result = subprocess.run(
                    [str(self.harness), str(image), '--cpu', 'pipeline',
                     '--music-stream', str(ROOT / 'tests/fixtures/furnace_minimal_v2.sgub'),
                     '--frames', '4', '--allow-running',
                     '--irq-drain-cycles', '40000000',
                     '--expect-mem-u8', '0xFFFF0045=1',
                     '--expect-mem-u8', '0x180FFFFF=128',
                     '--expect-mem-u8', '0x80020801=131',
                     '--expect-mem-u8', '0x80020803=255'],
                    cwd=self.work, capture_output=True, text=True, timeout=120)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn('PASS:', result.stdout)

    def test_irq_drain_rejects_handler_that_never_returns(self):
        source = self.work / 'stuck-irq.c'
        source.write_text('''
#include <cpt32/cpu.h>
#include <stdint.h>
void irqc_dispatch(void) { for (;;) {} }
int main(void) {
    *(volatile uint16_t *)0xFFFF0044u = 1;
    for (;;) cpt32_halt();
}
''')
        image = self.work / 'stuck-irq.wax'
        self.driver(source, '-I', ROOT / 'sdk/cpt32/include', '-o', image)
        for cpu in ('scalar', 'pipeline'):
            with self.subTest(cpu=cpu):
                result = subprocess.run(
                    [str(self.harness), str(image), '--cpu', cpu,
                     '--frames', '1', '--irq-drain-cycles', '800000',
                     '--allow-running', '--skip-render-checks'],
                    cwd=self.work, capture_output=True, text=True, timeout=120)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn('IRQ did not return within 800000 extra cycles', result.stderr)

    def test_fastcc_indirect_call_with_byval_and_wide_result(self):
        source = self.ir('''
%Pair = type { i32, i32 }
@pair = global %Pair { i32 6, i32 7 }
define internal fastcc i64 @helper(ptr byval(%Pair) %pair, i64 %base) {
  %a = load i32, ptr %pair
  %p = getelementptr %Pair, ptr %pair, i32 0, i32 1
  %b = load i32, ptr %p
  store i32 99, ptr %pair
  %product = mul i32 %a, %b
  %wide = zext i32 %product to i64
  %result = add i64 %base, %wide
  ret i64 %result
}
define i32 @main() {
  %target = select i1 true, ptr @helper, ptr @helper
  %result = call fastcc i64 %target(ptr byval(%Pair) @pair, i64 4294967296)
  %correct = icmp eq i64 %result, 4294967338
  %original = load i32, ptr @pair
  %private = icmp eq i32 %original, 6
  %ok = and i1 %correct, %private
  %answer = select i1 %ok, i32 42, i32 1
  ret i32 %answer
}
''')
        image = self.work / 'indirect-fastcc.wax'
        self.driver(source, '-o', image)
        self.execute(image, 42)
        self.execute(image, 42, 'pipeline')

    def test_other_calling_conventions_and_musttail_rejected(self):
        for convention, modifier in (('coldcc', ''), ('fastcc', 'musttail ')):
            with self.subTest(convention=convention, modifier=modifier):
                source = self.ir(f'''
define {convention} i32 @helper() {{ ret i32 42 }}
define {convention} i32 @main() {{
  %r = {modifier}call {convention} i32 @helper()
  ret i32 %r
}}
''')
                result = self.driver(source, '-o', self.work / 'bad.wax', success=False)
                self.assertIn('convention', result.stderr)

    def test_phi_parallel_copy_on_backedge(self):
        source = self.ir('''
define i32 @main() {
entry:
  br label %loop
loop:
  %a = phi i32 [ 1, %entry ], [ %b, %body ]
  %b = phi i32 [ 2, %entry ], [ %a, %body ]
  %n = phi i32 [ 0, %entry ], [ %next, %body ]
  %again = icmp ult i32 %n, 5
  br i1 %again, label %body, label %done
body:
  %next = add i32 %n, 1
  br label %loop
done:
  %tens = mul i32 %a, 10
  %result = add i32 %tens, %b
  ret i32 %result
}
''')
        image = self.work / 'phi.wax'
        self.driver(source, '-o', image)
        self.execute(image, 21)
        self.execute(image, 21, 'pipeline')

    def test_long_forward_jump_and_relocated_image(self):
        # Side effects keep the unreachable-by-branch block large. The taken
        # branch must jump beyond signed16 bytes without executing its stores.
        stores = '\n'.join('  store volatile i32 1, ptr @flag' for _ in range(1800))
        source = self.ir('''
@flag = global i32 0
define i32 @main() {
entry:
  %flag = load volatile i32, ptr @flag
  %skip = icmp eq i32 %flag, 0
  br i1 %skip, label %done, label %large
large:
''' + stores + '''
  br label %done
done:
  %result = load volatile i32, ptr @flag
  ret i32 %result
}
''')
        image = self.work / 'long.wax'
        self.driver(source, '--origin', '0x20000', '-o', image)
        self.execute(image, 0)
        self.execute(image, 0, 'pipeline')

    def test_i1_switch_true_and_false_cases(self):
        source = self.ir('''
define i32 @choose(i1 %value) {
entry:
  switch i1 %value, label %default [
    i1 true, label %yes
    i1 false, label %no
  ]
yes:
  ret i32 42
no:
  ret i32 17
default:
  ret i32 0
}
define i32 @main() {
entry:
  %yes = call i32 @choose(i1 true)
  %no = call i32 @choose(i1 false)
  %ok = icmp eq i32 %no, 17
  br i1 %ok, label %finish, label %fail
finish:
  ret i32 %yes
fail:
  ret i32 0
}
''')
        image = self.work / 'bool-switch.wax'
        self.driver(source, '-o', image)
        self.execute(image, 42)

    def test_names_are_not_abi_or_memory_attributes(self):
        source = self.ir('''
@atomic = global i32 42
define i32 @acc(i32 %x) {
  %result = add i32 %x, 1
  ret i32 %result
}
define i32 @byref(i32 %nest) {
  ret i32 %nest
}
define i32 @main() {
  %sum = call i32 @acc(i32 41)
  %same = call i32 @byref(i32 %sum)
  %loaded = load i32, ptr @atomic
  %ok = icmp eq i32 %same, %loaded
  %result = select i1 %ok, i32 %same, i32 0
  ret i32 %result
}
''')
        image = self.work / 'reserved-names.wax'
        self.driver(source, '-o', image)
        self.execute(image, 42)

    def test_incompatible_module_layouts_rejected_before_link(self):
        aligned = self.work / 'aligned.ll'
        packed = self.work / 'packed.ll'
        aligned.write_text('target datalayout = "E-p:32:32-i64:64-n32-S64"\n'
                           'target triple = "mips-unknown-none"\n'
                           'define i32 @main() { ret i32 42 }\n')
        packed.write_text('target datalayout = "E-p:32:32-i64:32-n32-S64"\n'
                          'target triple = "mips-unknown-none"\n'
                          '@object = global { i8, i64 } { i8 0, i64 42 }\n')
        result = self.driver(aligned, packed, '-o', self.work / 'bad.wax', success=False)
        self.assertIn('data layout differs', result.stderr)

    def test_undefined_external_diagnosed(self):
        source = self.ir('''
declare i32 @missing(i32)
define i32 @main() {
  %r = call i32 @missing(i32 3)
  ret i32 %r
}
''')
        result = self.driver(source, '-o', self.work / 'bad.wax', success=False)
        self.assertIn('missing', result.stderr)

    def test_c_defines_includes_and_ir_assembly_roundtrip(self):
        includes = self.work / 'include path'
        includes.mkdir()
        (includes / 'value.h').write_text('int supplied(void);\n')
        source = self.work / 'custom.c'
        source.write_text('#include <value.h>\n'
                          'int supplied(void) { return VALUE; }\n'
                          'int main(void) { return supplied(); }\n')
        linked = self.work / 'linked.ll'
        self.driver(source, '-I', includes, '-DVALUE=42', '--emit-llvm', '-o', linked)
        assembly = self.work / 'program.a'
        self.driver(linked, '-S', '-o', assembly)
        image = self.work / 'reassembled.wax'
        result = subprocess.run(
            [sys.executable, str(ROOT / 'tools/asm/asm.py'), str(assembly),
             '-o', str(image)], cwd=self.work, capture_output=True, text=True,
            timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.execute(image, 42)


    def test_cmake_library_and_header_rebuild(self):
        cmake = shutil.which('cmake')
        self.assertIsNotNone(cmake, 'Install CMake 3.20+')
        ninja = shutil.which('ninja')
        if not ninja:
            import ninja as ninja_package
            ninja = str(Path(ninja_package.BIN_DIR) / ('ninja.exe' if os.name == 'nt' else 'ninja'))
        source = self.work / 'cmake project'
        source.mkdir()
        (source / 'include path').mkdir()
        header = source / 'include path' / 'value.h'
        header.write_text('#define EXTRA 2\n')
        (source / 'logic.c').write_text('#include <value.h>\nint value(void) { return BASE + EXTRA; }\n')
        (source / 'main.c').write_text('int value(void);\nint main(void) { return value(); }\n')
        module = (HERE / 'cmake/SRC32.cmake').as_posix()
        (source / 'CMakeLists.txt').write_text(
            'cmake_minimum_required(VERSION 3.20)\n'
            'project(dependency_check LANGUAGES NONE)\n'
            f'include("{module}")\n'
            'src32_add_library(logic SOURCES logic.c INCLUDE_DIRECTORIES "include path" DEFINITIONS BASE=40)\n'
            'src32_add_executable(program SOURCES main.c LIBRARIES logic)\n')
        build = self.work / 'build path'

        def run(*args):
            result = subprocess.run(list(map(str, args)), cwd=self.work,
                                    capture_output=True, text=True, timeout=120)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            return result

        run(cmake, '-S', source, '-B', build, '-G', 'Ninja',
            f'-DCMAKE_MAKE_PROGRAM={ninja}',
            f'-DSRC32_PYTHON_EXECUTABLE={sys.executable}', f'-DSRC32_CLANG={self.clang}')
        run(cmake, '--build', build, '--target', 'program')
        self.execute(build / 'program.wax', 42)
        # Change semantics, not merely timestamps: rebuilt image must return 43.
        header.write_text('#define EXTRA 3\n')
        run(cmake, '--build', build, '--target', 'program')
        self.execute(build / 'program.wax', 43)

    def test_wrong_endianness_diagnosed(self):
        source = self.work / 'little.ll'
        source.write_text('target datalayout = "e-p:32:32"\n'
                          'define i32 @main() { ret i32 0 }\n')
        self.driver(source, '-o', self.work / 'bad.wax', success=False)

    def test_invalid_ir_diagnosed(self):
        source = self.ir('define i32 @main() { ret i8 0 }\n')
        self.driver(source, '-o', self.work / 'bad.wax', success=False)

    def test_duplicate_definitions_diagnosed(self):
        first = self.ir('define i32 @main() { ret i32 0 }\n', 'first.ll')
        second = self.ir('define i32 @main() { ret i32 1 }\n', 'second.ll')
        self.driver(first, second, '-o', self.work / 'bad.wax', success=False)


if __name__ == '__main__':
    unittest.main()
