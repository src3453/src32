#include <cpt32/mmio.h>
#include <cpt32/pec_rng.h>
void pec_rng_initPRNG(void) {
  uint32_t seed = CPT32_MMIO32(PEC_RNG_TRNG_ADDR);
  CPT32_MMIO32(PEC_RNG_PRNG_SEED_ADDR) = seed;
  (void)CPT32_MMIO32(PEC_RNG_PRNG_ADDR);
}
uint32_t pec_rng_rand(void) { return CPT32_MMIO32(PEC_RNG_PRNG_ADDR); }
uint32_t pec_rng_randT(void) { return CPT32_MMIO32(PEC_RNG_TRNG_ADDR); }
