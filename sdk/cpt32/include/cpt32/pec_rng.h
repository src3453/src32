#ifndef CPT32_PEC_RNG_H
#define CPT32_PEC_RNG_H
#include <stdint.h>
#define PEC_RNG_PRNG_ADDR 0x80040010u
#define PEC_RNG_PRNG_SEED_ADDR 0x80040014u
#define PEC_RNG_TRNG_ADDR 0x80040018u
/* TRNGの値でPRNGを初期化し、最初の読出し値を破棄します。 */
void pec_rng_initPRNG(void);
/* PRNGから32-bitの擬似乱数を1つ取得します。 */
uint32_t pec_rng_rand(void);
/* TRNGから32-bitの真性乱数を1つ取得します。 */
uint32_t pec_rng_randT(void);
#endif
