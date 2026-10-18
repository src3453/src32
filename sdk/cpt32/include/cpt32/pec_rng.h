#ifndef CPT32_PEC_RNG_H
#define CPT32_PEC_RNG_H
#include <stdint.h>
#define PEC_RNG_PRNG_ADDR 0x80040010u
#define PEC_RNG_PRNG_SEED_ADDR 0x80040014u
#define PEC_RNG_TRNG_ADDR 0x80040018u
void pec_rng_initPRNG(void);
uint32_t pec_rng_rand(void);
uint32_t pec_rng_randT(void);
#endif
