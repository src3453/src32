!include "sol/cpt32/pcg.sol"

initPCG col40
0xc colorfg "CPT32 POST v0.1 (C) 2026 src3453" prnlnscr
_pcg_ln
0x3f colorfg "CPU: SRC32-ALMSI (rev 2.2) @48MHz" prnlnscr
"RAM: 16384KB" prnlnscr
"VRAM: 4096KB" prnlnscr
"PCMRAM: 1024KB" prnlnscr
"Subsys:" prnlnscr
"  VDP (rev 0.2)" prnlnscr
"  PeC (rev 0.3)" prnlnscr
"  SGU (rev 0.1; S3W2)" prnlnscr
0x15 colorfg
"  VPU (Not installed)" prnlnscr
"  SC (Not installed)" prnlnscr
"  DMAC (Not installed)" prnlnscr
"  IRQC (Not installed)" prnlnscr
_pcg_ln 
0xc colorfg
"POST OK" prnlnscr
0x3f colorfg

!var i 0
while
    i colorbg
    " " prnscr
    i 1 add >i
    i 16 mod if
        _pcg_ln
    end
    i 0x40 neq
end
0 colorfg
"IPL is loading program..." prnlnscr