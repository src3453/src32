# Tang stage1 error recovery

- A root FS2 write sets DIRTY; sync is synchronous but does not clear the flag.
  Resetting the mounted root therefore makes stage1 reject the next boot.
  Preserve the volume and run the complete offline host FS2 check before
  clearing DIRTY. Never bypass the stage1 check or clear it merely after mount.
- Tang BIOS now passes its UART recovery callback in R26; Tang stage1 must
  preserve it and jump there on every explicit failure, with R1=1/2/3 for
  superblock read/check/FNDBOOT. Install BIOS and stage1 together. Standard
  emulator stage1 remains unchanged. This does not recover kernel faults.
- Sibling `src32-sv/scripts/sd_fsck.py` saves the original volume before checking
  it and can clear only DIRTY after full validation and sector read-back.
- Sibling `python scripts/run_idc_boot_sim.py` tests actual CPU/BIOS/native SD/
  stage1 success and failure recovery, including identify/read/retry afterwards.
- Validation: command/automatic boot, bad signature, DIRTY, bad superblock,
  bad FNDBOOT, superblock CRC failure and FNDBOOT CRC failure all passed.
  `--recovery-video` matched all 307,200 RGB pixels against the freshly built
  emulator replay, excluding no accepted transactions. Recovery-tool unit tests
  passed all four cases (clean, DIRTY, allocation corruption, unknown flags).
- Actual COM7 card had FS2 flags=1 and total_blocks=16384. Its original 8 MiB
  image is preserved at sibling `src32-sv/build/sd-before-recovery-20261028.img`.
  Full host FS2 validation passed with DIRTY allowed; only byte 519 was changed
  when clearing DIRTY, confirmed by live sector 0/1 read-back.
- New BIOS build: sibling `build/stage1-recovery/prj/soc_top/impl/pnr/soc_top.fs`,
  Gowin SP2 placement 2, 0.8 ns setup uncertainty, clk_sys 48.071 MHz against
  48 MHz; 46,784 LUT+ALU, 265 BSRAM and 24 DSP. BIOS uses 3,856 padded bytes.
- Physical board: programmed the new image to SRAM only and read all 3,853
  actual BIOS file bytes back from 0xFF000000, matching the assembled file.
  Replaced only SD sector 0 with the new stage1 (read-back verified). Set DIRTY
  temporarily twice: both boots printed `Stage1 error 2 - UART recovery` and
  returned to `boot>`, where IDENTIFY and sector reads worked. Restored the
  clean superblock with verified read-back, then reached Foundation OS/shell
  banners by booting from SD. The original files were retained.
- The SD-booted shell answered `uname` and a standalone echo marker. Send
  commands individually and wait for the prompt: queuing `uname` plus echo at
  10 ms/byte lost the tail while loading from SD. Ctrl-C cleared the partial
  line; a standalone command at 100 ms/byte completed. Evidence is in sibling
  `build/stage1-recovery-board.log` and `build/stage1-shell-check.bin`.
