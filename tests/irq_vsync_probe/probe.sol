# Manual PeC/IRQC regression probe for slow-frame testing.
#
# Build from the repository root:
#   python tools/solc/solc.py compile tests/irq_vsync_probe/probe.sol -o tests/irq_vsync_probe/probe.a -O speed
#   python tools/asm/asm.py tests/irq_vsync_probe/probe.a -o tests/irq_vsync_probe/probe.bin
#
# Launch from tests/irq_vsync_probe with a BIOS-free config.yaml:
#   $env:CPT32_PEC_TRACE = "1"
#   & ..\..\target\debug\cpt32.exe --debug-gui probe.bin 2> irq-vsync.log
# In Debug Controls, keep Run CPU checked and set Cycles / frame to 800.
# Press A at each prompt. The second prompt follows a busy loop spanning many
# VSyncs; compare the screen markers with FIFO/IRQC transitions in the log.

!include "../../sol/cpt32/pcg.sol"
!include "../../sol/cpt32/irqc.sol"

!rodata_base 0x00040000
!const PEC_DATA 0x80040030
!const PEC_STATUS 0x80040034
!const PEC_POP 0x80040038

!var probe_event 0
!var probe_work 0

fn irq3 () :
    8 irqc.IRQC_PENDING sth
    retn
;

# Drain startup pad/device events so source 3 is low before the first key edge.
fn drain_events () :
    while
        PEC_STATUS ld 1 and 0 gt
        if
            1 PEC_POP st
            0
        else
            1
        end
    end
;

# Wait through HALT for a physical A key-down (HID usage 4).
fn wait_for_a () :
    while
        PEC_STATUS ld 1 and 0 gt
        if
            PEC_DATA ld >probe_event
            # Comparisons use 0=true; OR combines predicates as logical AND.
            probe_event 28 shr 15 and 0 eq
            probe_event 24 shr 15 and 0 eq or
            probe_event 16 shr 255 and 4 eq or
            probe_event 1 and 1 eq or
            if
                1 PEC_POP st
                1 ret
            end
            1 PEC_POP st
            0
        else
            halt
            0
        end
    end
    0 ret
;

fn main () :
    1 0x80000000 stb
    pcg.initPCG
    pcg.col40
    drain_events
    8 irqc.IRQC_PENDING sth
    8 irqc.IRQC_ENABLE sth

    "BASELINE: PRESS A" pcg.prnlnscr
    wait_for_a
    "BASELINE KEY OK" pcg.prnlnscr

    0 >probe_work
    while
        probe_work 10000 lt
        if
            probe_work 1 add >probe_work
            0
        else
            1
        end
    end

    "STRESS DONE: PRESS A" pcg.prnlnscr
    wait_for_a
    "POST-STRESS KEY OK" pcg.prnlnscr
    halt
;

main
