!include "sol/solstd/solstd_print.sol"

!var i 0
@loop
"Hello, World!" prnln
i 1 add >i
100 i sub jnz @loop