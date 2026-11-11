/* The benchmark kernels in C. rust/src/lib.rs implements the same algorithms;
 * keep the two in step (see ../README.md). Helpers carry the bench_<kernel>
 * prefix so that run.py can attribute their code size to the kernel. */
#include <stdint.h>

#include "bench.h"

uint32_t bench_crc32(uint32_t *buffer, uint32_t n) {
  const uint8_t *bytes = (const uint8_t *)buffer;
  uint32_t table[256];
  for (uint32_t i = 0; i < 256u; ++i) {
    uint32_t c = i;
    for (int k = 0; k < 8; ++k)
      c = (c & 1u) ? 0xEDB88320u ^ (c >> 1) : c >> 1;
    table[i] = c;
  }
  uint32_t crc = 0xFFFFFFFFu;
  for (uint32_t i = 0; i < n; ++i)
    crc = table[(crc ^ bytes[i]) & 0xFFu] ^ (crc >> 8);
  return ~crc;
}

uint32_t bench_sieve(uint32_t *buffer, uint32_t n) {
  uint8_t *flags = (uint8_t *)buffer;
  for (uint32_t i = 0; i < n; ++i)
    flags[i] = 1;
  flags[0] = 0;
  flags[1] = 0;
  for (uint32_t i = 2; i * i < n; ++i)
    if (flags[i])
      for (uint32_t j = i * i; j < n; j += i)
        flags[j] = 0;
  uint32_t count = 0;
  for (uint32_t i = 0; i < n; ++i)
    count += flags[i];
  return count;
}

/* Hoare partition of a[lo..=hi] around the middle element; returns j with
 * lo <= j < hi, everything in a[lo..=j] <= everything in a[j+1..=hi]. */
static uint32_t bench_sort_partition(uint32_t *a, uint32_t lo, uint32_t hi) {
  uint32_t pivot = a[lo + (hi - lo) / 2u];
  uint32_t i = lo;
  uint32_t j = hi;
  for (;;) {
    while (a[i] < pivot)
      ++i;
    while (a[j] > pivot)
      --j;
    if (i >= j)
      return j;
    uint32_t t = a[i];
    a[i] = a[j];
    a[j] = t;
    ++i;
    --j;
  }
}

static void bench_sort_insertion(uint32_t *a, uint32_t lo, uint32_t hi) {
  for (uint32_t i = lo + 1u; i <= hi; ++i) {
    uint32_t value = a[i];
    uint32_t j = i;
    while (j > lo && a[j - 1u] > value) {
      a[j] = a[j - 1u];
      --j;
    }
    a[j] = value;
  }
}

/* Recurses into the smaller part and loops on the larger one. */
static void bench_sort_range(uint32_t *a, uint32_t lo, uint32_t hi) {
  while (hi - lo >= 16u) {
    uint32_t middle = bench_sort_partition(a, lo, hi);
    if (middle - lo < hi - middle) {
      bench_sort_range(a, lo, middle);
      lo = middle + 1u;
    } else {
      bench_sort_range(a, middle + 1u, hi);
      hi = middle;
    }
  }
  bench_sort_insertion(a, lo, hi);
}

uint32_t bench_sort(uint32_t *buffer, uint32_t n) {
  if (n > 1u)
    bench_sort_range(buffer, 0, n - 1u);
  uint32_t sum = 0;
  for (uint32_t i = 0; i < n; ++i) {
    if (i > 0 && buffer[i - 1u] > buffer[i])
      return 0xFFFFFFFFu;
    sum += buffer[i] * (i + 1u);
  }
  return sum;
}

uint32_t bench_matmul(uint32_t *buffer, uint32_t n) {
  const int32_t *a = (const int32_t *)buffer;
  const int32_t *b = a + n * n;
  int32_t *c = (int32_t *)buffer + 2u * n * n;
  for (uint32_t i = 0; i < n; ++i)
    for (uint32_t j = 0; j < n; ++j) {
      uint32_t sum = 0; /* wrapping, as the Rust version */
      for (uint32_t k = 0; k < n; ++k)
        sum += (uint32_t)a[i * n + k] * (uint32_t)b[k * n + j];
      c[i * n + j] = (int32_t)sum;
    }
  uint32_t hash = 0;
  for (uint32_t i = 0; i < n * n; ++i)
    hash = hash * 31u + (uint32_t)c[i];
  return hash;
}

static uint32_t bench_nqueens_solve(uint32_t columns, uint32_t left,
                                    uint32_t right, uint32_t all) {
  if (columns == all)
    return 1;
  uint32_t count = 0;
  uint32_t free = all & ~(columns | left | right);
  while (free != 0) {
    uint32_t bit = free & (0u - free);
    free -= bit;
    count += bench_nqueens_solve(columns | bit, (left | bit) << 1,
                                 (right | bit) >> 1, all);
  }
  return count;
}

uint32_t bench_nqueens(uint32_t *buffer, uint32_t n) {
  (void)buffer;
  return bench_nqueens_solve(0, 0, 0, (1u << n) - 1u);
}

uint32_t bench_mandel(uint32_t *buffer, uint32_t n) {
  (void)buffer;
  const int32_t one = 4096; /* Q12 */
  int32_t width = (int32_t)n;
  int32_t height = width * 2 / 3;
  uint32_t total = 0;
  for (int32_t y = 0; y < height; ++y) {
    int32_t ci = -one + 2 * one * y / height;
    for (int32_t x = 0; x < width; ++x) {
      int32_t cr = -10 * one / 4 + 14 * one / 4 * x / width;
      int32_t zr = 0;
      int32_t zi = 0;
      uint32_t iteration = 0;
      while (iteration < 32u) {
        int32_t zr2 = (zr * zr) >> 12;
        int32_t zi2 = (zi * zi) >> 12;
        if (zr2 + zi2 > 4 * one)
          break;
        zi = ((zr * zi) >> 11) + ci;
        zr = zr2 - zi2 + cr;
        ++iteration;
      }
      total += iteration;
    }
  }
  return total;
}
