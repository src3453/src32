!var PRNG_ADDR 0x80040010
!var PRNG_SEED_ADDR 0x80040014
!var TRNG_ADDR 0x80040018

fn initPRNG () :
    TRNG_ADDR ld
    PRNG_SEED_ADDR st
    PRNG_ADDR ld drop
;

fn rand () :
    PRNG_ADDR ld ret
;

fn randT () :
   TRNG_ADDR ld ret 
;