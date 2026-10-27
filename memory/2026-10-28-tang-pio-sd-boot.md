# Tang PIO SD boot

- `--platform tang-console --root-drive microsd` builds a dd0 root. The default
  remains dd1 for existing UART-loaded RAM-disk manifests. Standard cpt32 boot
  behavior and the DMA stage1 are unchanged.
- Tang images use a real 512-byte PIO stage1. The BIOS must IDENTIFY the drive
  before reading and entering its signed LBA 0, passing drive in R1, SP=0xF0000,
  IRQs disabled. The stage1 obtains capacity from that IDENTIFY result.
- The PIO read routine must preserve R5: superblock validation uses its original
  0x200 address after read returns. Increment a separate destination register.
  Keep every existing FS2/FNDBOOT range/header check when fitting the sector.
- The FPGA BIOS has a one-second UART recovery window, automatic dd0 boot and a
  B retry command. Its PCG output uses the same UNSCII font/CLUT as Foundation;
  binary UART replies bypass the text renderer.
- Checks: `python tools/fs2/fs2.py check target/foundation-sd-vram/foundation.img`;
  in the sibling RTL tree `python scripts/run_idc_boot_sim.py` exercises actual
  ROM/CPU/native SD/X2 SDRAM/Tang stage1 through a synthetic kernel entry, plus
  bad-signature recovery and automatic selection. Only the one-second timer
  wait is fast-forwarded. `--video-only` compares the actual BIOS framebuffer
  with a fresh emulator replay of its accepted VRAM/MMIO writes.
- A bootable FS2 superblock must be clean. Full offline fs2.check is required
  before clearing DIRTY after filesystem writes; a successful mount is insufficient.
