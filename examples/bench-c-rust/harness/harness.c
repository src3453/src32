/* Measurement harness shared by the C and the Rust build (run.py links this
 * same file, at the same -O level, into both), so that only the kernels differ.
 *
 * For every kernel it prepares the input, paints the stack below the caller,
 * reads the PeC 48 MHz timer (CPU system clock cycles) around the call and
 * scans for the lowest overwritten stack word. Results go to the UART:
 *
 *   INFO overhead <cycles of two back-to-back timer reads>
 *   INFO paint <lowest painted address> <highest painted address>
 *   INFO buffer <first address> <end address>
 *   RESULT <name> <cycles> <stack bytes> <checksum>
 *   DONE
 *
 * Every kernel runs twice on freshly prepared input; the second run is
 * reported. main returns 0. */
#include <cpt32/mmio.h>
#include <cpt32/print.h>
#include <stdint.h>

#include "bench.h"

#define TIMER_ENABLE_ADDR 0x80040184u
#define TIMER0_CTRL_ADDR 0x80040400u
#define TIMER0_COUNT_LO_ADDR 0x80040408u
#define TIMER0_PRESCALER_ADDR 0x80040420u
#define TIMER_CTRL_FREE_RUNNING 0x2u
#define TIMER_CTRL_ENABLE 0x4u
#define TIMER_CTRL_RESET 0x10u

#define BUFFER_WORDS 8192u
#define PAINT_WORDS 8192u /* 32 KiB below the caller of the kernel */
#define PAINT_PATTERN 0xA5C3E1F7u
#define PAINT_MARGIN 1024u
/* The kernel runs inside a frame this large, so that it starts below the top
 * of the painted area (paint()'s own frame and PAINT_MARGIN lie above it). */
#define DESCENT_BYTES 4096u

static uint32_t buffer[BUFFER_WORDS];
static uint32_t *paint_low;
static uint32_t *paint_high;
static uint32_t lcg_state;

static uint32_t lcg(void) {
  lcg_state = lcg_state * 1664525u + 1013904223u;
  return lcg_state;
}

static uint32_t timer_read(void) { return CPT32_MMIO32(TIMER0_COUNT_LO_ADDR); }

static void timer_start(void) {
  CPT32_MMIO32(TIMER0_PRESCALER_ADDR) = 0;
  CPT32_MMIO32(TIMER_ENABLE_ADDR) = 1;
  CPT32_MMIO32(TIMER0_CTRL_ADDR) =
      TIMER_CTRL_RESET | TIMER_CTRL_ENABLE | TIMER_CTRL_FREE_RUNNING;
}

static void put_decimal(uint32_t value) {
  char digits[10];
  int length = 0;
  do {
    digits[length++] = (char)('0' + value % 10u);
    value /= 10u;
  } while (value != 0);
  while (length > 0)
    print_putc((uint32_t)digits[--length]);
}

static void put_field(uint32_t value) {
  print_putc(' ');
  put_decimal(value);
}

/* Paints PAINT_WORDS words that end PAINT_MARGIN bytes below this frame. */
__attribute__((noinline)) static void paint(void) {
  volatile uint32_t marker = 0;
  uintptr_t top = ((uintptr_t)&marker - PAINT_MARGIN) & ~(uintptr_t)3;
  uint32_t *high = (uint32_t *)top;
  uint32_t *low = high - PAINT_WORDS;
  for (uint32_t *word = low; word < high; ++word)
    *word = PAINT_PATTERN;
  paint_low = low;
  paint_high = high;
}

/* Bytes between `from` and the lowest overwritten painted word, or
 * 0xFFFFFFFF when even the lowest painted word was overwritten. */
static uint32_t stack_depth(uintptr_t from) {
  if (*paint_low != PAINT_PATTERN)
    return 0xFFFFFFFFu;
  const uint32_t *word = paint_low;
  while (word < paint_high && *word == PAINT_PATTERN)
    ++word;
  return (uint32_t)(from - (uintptr_t)word);
}

static uint32_t measured_cycles;
static uint32_t measured_stack;

/* Calls the kernel from inside a DESCENT_BYTES frame; the kernel's frames are
 * all below `descent`. */
__attribute__((noinline)) static uint32_t run_kernel(bench_kernel kernel,
                                                     uint32_t n) {
  volatile uint8_t descent[DESCENT_BYTES];
  descent[0] = 0;
  uint32_t start = timer_read();
  uint32_t value = kernel(buffer, n);
  uint32_t end = timer_read();
  measured_cycles = end - start;
  measured_stack = stack_depth((uintptr_t)&descent[0]);
  descent[0] = (uint8_t)value;
  return value;
}

__attribute__((noinline)) static uint32_t empty_kernel(uint32_t *b,
                                                       uint32_t n) {
  (void)b;
  (void)n;
  return 0;
}

enum { CRC32, SIEVE, SORT, MATMUL, NQUEENS, MANDEL, EMPTY };

static void prepare(int which) {
  uint8_t *bytes = (uint8_t *)buffer;
  switch (which) {
  case CRC32:
    lcg_state = 1;
    for (uint32_t i = 0; i < 16384u; ++i)
      bytes[i] = (uint8_t)(lcg() >> 24);
    break;
  case SORT:
    lcg_state = 2;
    for (uint32_t i = 0; i < 2000u; ++i)
      buffer[i] = lcg();
    break;
  case MATMUL:
    lcg_state = 3;
    for (uint32_t i = 0; i < 2u * 24u * 24u; ++i)
      buffer[i] = (uint32_t)((int32_t)(lcg() >> 28) - 8);
    break;
  default: /* the sieve initialises its flags itself */
    break;
  }
}

static void measure(const char *name, int which, bench_kernel kernel,
                    uint32_t n) {
  uint32_t value = 0;
  for (int run = 0; run < 2; ++run) {
    prepare(which);
    paint();
    value = run_kernel(kernel, n);
  }
  print_prn("RESULT ");
  print_prn(name);
  put_field(measured_cycles);
  put_field(measured_stack);
  put_field(value);
  print_prnln("");
}

int main(void) {
  timer_start();
  uint32_t first = timer_read();
  uint32_t second = timer_read();
  print_prn("INFO overhead");
  put_field(second - first);
  print_prnln("");
  paint();
  print_prn("INFO paint");
  put_field((uint32_t)(uintptr_t)paint_low);
  put_field((uint32_t)(uintptr_t)paint_high);
  print_prnln("");
  print_prn("INFO buffer");
  put_field((uint32_t)(uintptr_t)buffer);
  put_field((uint32_t)(uintptr_t)(buffer + BUFFER_WORDS));
  print_prnln("");

  measure("empty", EMPTY, empty_kernel, 0);
  measure("crc32", CRC32, bench_crc32, 16384u);
  measure("sieve", SIEVE, bench_sieve, 32768u);
  measure("sort", SORT, bench_sort, 2000u);
  measure("matmul", MATMUL, bench_matmul, 24u);
  measure("nqueens", NQUEENS, bench_nqueens, 9u);
  measure("mandel", MANDEL, bench_mandel, 48u);
  print_prnln("DONE");
  return 0;
}
