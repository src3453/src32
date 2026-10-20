#ifndef CPT32_CPU_H
#define CPT32_CPU_H

/* Emit the SRC32 HALT instruction. The CPU resumes after the call when an
 * enabled IRQ is accepted; an IRET returns to the following instruction.
 * Requires tools/llvm-src32; this is not a host-side CPU or sleep function. */
/* HALT命令でCPUを停止し、有効なIRQの受理後に再開します。 */
void cpt32_halt(void);

#endif
