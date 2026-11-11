/* The benchmark kernels, implemented once in C (c/kernels.c) and once in Rust
 * (rust/src/lib.rs). Each kernel takes the shared input buffer prepared by
 * harness.c and a size, and returns a checksum that run.py compares with its
 * Python reference. The algorithms and data layouts must stay identical in
 * both languages; see README.md for the exact definitions. */
#ifndef BENCH_H
#define BENCH_H
#include <stdint.h>

typedef uint32_t (*bench_kernel)(uint32_t *buffer, uint32_t n);

/* CRC-32 (IEEE, as zlib.crc32) of n bytes; builds its 256-entry table on the stack. */
uint32_t bench_crc32(uint32_t *buffer, uint32_t n);
/* Sieve of Eratosthenes over n byte flags; returns the number of primes below n. */
uint32_t bench_sieve(uint32_t *buffer, uint32_t n);
/* Quicksort (Hoare partition, insertion sort below 16) of n words; returns
 * sum(a[i] * (i + 1)), or 0xFFFFFFFF if the result is not sorted. */
uint32_t bench_sort(uint32_t *buffer, uint32_t n);
/* C = A * B for n x n int32 matrices A, B, C stored one after another;
 * returns h = h * 31 + C[i] over C. */
uint32_t bench_matmul(uint32_t *buffer, uint32_t n);
/* Solutions of the n-queens problem (recursive bitmask search). */
uint32_t bench_nqueens(uint32_t *buffer, uint32_t n);
/* Q12 fixed-point Mandelbrot on an n x (n * 2 / 3) grid, at most 32
 * iterations; returns the sum of the iteration counts. */
uint32_t bench_mandel(uint32_t *buffer, uint32_t n);

#endif
