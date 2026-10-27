# Tang MicroSD dd0 / PIO IDC

- Tang RTL now exposes MicroSD at IDC slot 0 with standard offsets and byte DATA PIO. Hardware +0x3c bit0 advertises PIO; there is still no DMAC. The emulator remains the DMA reference and its SDK DMA helpers cannot be used on the Tang profile.
- `FND_PLATFORM_TANG` routes dd0 through PIO, a 512-byte kernel buffer and `mm_copy_*`. dd1 retains the separate SDRAM disk window. UART-loaded Tang boot explicitly selects boot drive 1, preserving the RAM-disk root while a valid FS2 card mounts at `/dd/0`.
- Rebuild old Tang kernels: an old kernel interprets dd0 as SDRAM. Other platform builds retain the existing DMA/IRQ backend.
- `foundation/tests/console/ramdisk_host.c` now checks both dd1 boundaries/access checks and dd0 identify, 2-sector PIO, virtual-user copies and error translation. Run `python -m unittest discover -s foundation/tests/console -p test_headless.py`.
- Build checked with `python foundation/boot/build.py target/foundation-microsd --platform tang-console -j 4`. On Windows sandboxed Clang needs workspace-local TEMP/TMP; native Icarus/Gowin may require normal host execution.
- See adjacent RTL repository `doc/microsd.md` for SPI details, BIOS raw protocol, exact emulator comparison exclusions and physical test procedure. This change does not add SD auto-boot or alter the standard DMA-based stage1.
