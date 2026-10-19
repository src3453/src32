#ifndef CPT32_CPU_H
#define CPT32_CPU_H

/* Emit the SRC32 HALT instruction. The CPU resumes after the call when an
 * enabled IRQ is accepted; an IRET returns to the following instruction.
 * Requires tools/llvm-src32; this is not a host-side CPU or sleep function. */
void cpt32_halt(void);

#endif
