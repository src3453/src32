# IRQC registers (16-bit pending and enable masks, big-endian MMIO)
!var IRQC_PENDING 0xFFFF0040
!var IRQC_ENABLE 0xFFFF0044

# Return the lowest numbered enabled pending IRQ, or -1 when none is active.
fn irqc_irq_number () :
    local active 0
    local number 0
    IRQC_PENDING ld
    IRQC_ENABLE ld
    and >active
    while
        active 0 eq
        if
            -1 ret
        end
        active 1 and
        if
            number ret
        end
        active 1 shr >active
        number 1 add >number
        active 0 eq
    end
;
