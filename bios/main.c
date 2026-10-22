/* CPT32 BIOS (C): POST hardware survey followed by the boot loader.
 *
 * Built as a ROM image at 0xFF000000 with writable data in the BIOS work
 * area (see bios/README.md and doc/spec_BIOS.md). Boot order:
 *   1. RAM   : a program already loaded at 0x00000000 (first word != 0)
 *   2. ROM   : reserved, not implemented yet (always skipped)
 *   3. IDC   : dd0, dd1, dd2, dd3 boot sector (LBA 0, 512 bytes, 0x55 0xAA)
 *
 * Survey policy (same as sol/bios.sol): only touch registers that are mapped
 * in every standard configuration. Unmapped access raises a bus error, so
 * UART, PeC timer/RTC and WASM expansion slots are not probed. */
#include <cpt32/bmc.h>
#include <cpt32/cpu.h>
#include <cpt32/idc.h>
#include <cpt32/irqc.h>
#include <cpt32/mmio.h>
#include <cpt32/pcg.h>
#include <stdint.h>

#define POST_HOLD_FRAMES 120u
#define BOOT_ENTRY 0x00000000u
#define BOOT_SECTOR_SIZE 512u
#define BOOT_SIGNATURE_OFFSET 0x1FEu
#define BOOT_STACK_TOP 0x000F0000u /* bottom of the BIOS work area */
#define BOOT_DEVICE_RAM 0xFFFFFFFFu
#define IDC_DRIVES 4u
#define RAM_TEST_FALLBACK 0x00F00000u /* BMC without memory size registers */

#define COLOR_TITLE 0x0Cu
#define COLOR_TEXT 0x3Fu
#define COLOR_OK 0x2Au
#define COLOR_NG 0x15u

static uint32_t fails;

static void newline(void) { pcg_prnlnscr(""); }

static void print_hex_digit(uint32_t n) {
  pcg_putcscr(n < 10u ? n + 0x30u : n + 0x37u);
}

static void print_hex8(uint32_t value) {
  for (int shift = 28; shift >= 0; shift -= 4)
    print_hex_digit((value >> shift) & 0xFu);
}

static void print_hex2(uint32_t value) {
  print_hex_digit((value >> 4) & 0xFu);
  print_hex_digit(value & 0xFu);
}

/* Small decimal without division (values here stay below 10^4). */
static void print_dec(uint32_t value) {
  static const uint32_t powers[] = {1000u, 100u, 10u, 1u};
  int started = 0;
  for (uint32_t i = 0; i < 4u; i++) {
    uint32_t digit = 0;
    while (value >= powers[i]) {
      value -= powers[i];
      digit++;
    }
    if (digit || started || i == 3u) {
      print_hex_digit(digit);
      started = 1;
    }
  }
}

/* Capacity as <n>M, or <n>K when not a whole MiB; "?" when unreported. */
static void print_size(const char *label, uint32_t bytes) {
  pcg_prnscr(label);
  if (bytes == 0u)
    pcg_prnscr("?");
  else if ((bytes & 0xFFFFFu) == 0u) {
    print_dec(bytes >> 20);
    pcg_prnscr("M");
  } else {
    print_dec(bytes >> 10);
    pcg_prnscr("K");
  }
}

static void print_colored(uint32_t color, const char *text) {
  pcg_colorfg(color);
  pcg_prnscr(text);
  pcg_colorfg(COLOR_TEXT);
}

static void verdict(int passed) {
  if (passed) {
    print_colored(COLOR_OK, "OK");
  } else {
    print_colored(COLOR_NG, "NG");
    fails++;
  }
}

/* Print value as 8 hex digits, then OK/NG against expect. */
static void report(const char *label, uint32_t value, uint32_t expect) {
  pcg_prnscr(label);
  print_hex8(value);
  pcg_prnscr(" ");
  verdict(value == expect);
  newline();
}

/* 32-bit RAM R/W test with save/restore. */
static int ram_test32(uint32_t address) {
  uint32_t saved = CPT32_MMIO32(address);
  CPT32_MMIO32(address) = 0x5A5A5A5Au;
  int passed = CPT32_MMIO32(address) == 0x5A5A5A5Au;
  CPT32_MMIO32(address) = saved;
  return passed;
}

/* SGU wavetable byte R/W test with save/restore (proves mapping). */
static int sgu_test(uint32_t address) {
  uint8_t saved = CPT32_MMIO8(address);
  CPT32_MMIO8(address) = 0xA5u;
  int passed = CPT32_MMIO8(address) == 0xA5u;
  CPT32_MMIO8(address) = saved;
  return passed;
}

static void post(void) {
  pcg_colorfg(COLOR_TITLE);
  pcg_prnlnscr("CPT32 BIOS v0.5 POST+BOOT (C)");
  pcg_colorfg(COLOR_TEXT);
  pcg_prnlnscr("CPU: SRC32 @48MHz");
  uint32_t ram = bmc_main_ram_bytes();
  print_size("MEM: RAM", ram);
  print_size(" VRAM", bmc_vram_bytes());
  print_size(" PCM", bmc_pcmram_bytes());
  newline();

  uint32_t vdp = CPT32_MMIO8(0x80000002u);
  pcg_prnscr("VDP status=");
  print_hex2(vdp);
  pcg_prnscr(" ");
  verdict(vdp == 0u);
  newline();

  report("SGC id=", CPT32_MMIO32(0x80010000u), 0x53474331u);
  uint32_t sgc = CPT32_MMIO32(0x80010008u);
  pcg_prnscr("SGC status=");
  print_hex8(sgc);
  pcg_prnscr(" ");
  verdict((sgc & 0x1Eu) == 0u);
  newline();

  report("VPU id=", CPT32_MMIO32(0x80030000u), 0x56505531u);
  uint32_t vpu = CPT32_MMIO32(0x80030004u);
  pcg_prnscr("VPU status=");
  print_hex8(vpu);
  pcg_prnscr(" ");
  verdict((vpu & 0x2Cu) == 0u);
  newline();

  pcg_prnscr("SGU0 wave=");
  verdict(sgu_test(0x80020000u));
  pcg_prnscr(" SGU1 wave=");
  verdict(sgu_test(0x80021000u));
  newline();

  report("PeC seed=", CPT32_MMIO32(0x80040014u), 0x55555555u);
  pcg_prnscr("PeC trng=");
  print_hex8(CPT32_MMIO32(0x80040018u));
  pcg_prnscr(" ev=");
  print_hex8(CPT32_MMIO32(0x80040034u));
  newline();

  report("DMAC id=", CPT32_MMIO32(0x80050000u), 0x444D4101u);
  report("BMC id=", CPT32_MMIO32(0x80060000u), 0x424D4301u);
  report("IDC id=", CPT32_MMIO32(0x80041000u), 0x49444301u);
  pcg_prnscr("IDC cap=");
  print_hex8(CPT32_MMIO32(0x80041030u));
  pcg_prnscr(" sz=");
  print_hex8(CPT32_MMIO32(0x8004102Cu));
  newline();

  /* IRQC pending/enable is event-driven (VBlank etc. fire during POST), so
   * a nonzero value is normal. Display only, never fail. */
  pcg_prnscr("IRQC=");
  print_hex8(CPT32_MMIO32(IRQC_PENDING));
  newline();

  /* 1 MiB below the top of RAM (0x00F00000 on 16 MiB), clear of the BIOS
   * work area and of small host-loaded programs. */
  uint32_t ram_test = ram == 0u ? RAM_TEST_FALLBACK
                      : ram > 0x00200000u ? ram - 0x00100000u
                                          : ram >> 1;
  pcg_prnscr("RAM=");
  verdict(ram_test32(ram_test));
  pcg_prnscr(" VRAM=");
  verdict(ram_test32(0x10300000u));
  newline();
  pcg_prnlnscr("UART/TIMER/EXP: skipped");

  if (fails == 0u)
    print_colored(COLOR_TITLE, "POST OK");
  else
    print_colored(COLOR_NG, "POST FAIL");
  newline();

  for (uint32_t i = 0; i < 16u; i++) {
    pcg_colorbg(i);
    pcg_prnscr(" ");
  }
  pcg_colorbg(0);
  newline();
}

/* One host frame runs the whole POST, so without waiting the survey is never
 * rendered. Each VSync (IRQ0) comes from the host frame loop. */
static void hold_post(void) {
  pcg_prnlnscr("BOOT: hold POST 120 frames...");
  irqc_init();
  irqc_enable(1u);
  for (uint32_t i = 0; i < POST_HOLD_FRAMES; i++)
    irqc_wait_vsync();
  irqc_disable(0xFFFFu);
  irqc_ack(0xFFFFu);
}

/* Disable and acknowledge every IRQ source, then jump with R1 = device.
 * The IRQ vectors still point at the BIOS dispatcher; the next stage must
 * install its own before enabling sources. */
static _Noreturn void boot(uint32_t device) {
  irqc_disable(0xFFFFu);
  irqc_ack(0xFFFFu);
  cpt32_jump(BOOT_ENTRY, device, BOOT_STACK_TOP);
}

/* Address arithmetic through a parameter keeps the load of RAM 0 a plain
 * volatile access instead of a constant null dereference. */
static uint32_t ram_word(uint32_t address) { return CPT32_MMIO32(address); }
static uint32_t ram_byte(uint32_t address) { return CPT32_MMIO8(address); }

static void clear_boot_sector(void) {
  for (uint32_t offset = 0; offset < BOOT_SECTOR_SIZE; offset += 4u)
    CPT32_MMIO32(BOOT_ENTRY + offset) = 0u;
}

/* Returns only when the drive is not bootable; RAM 0..511 is cleared again
 * so a failed attempt never looks like a RAM program on the next reset. */
static void try_idc(uint32_t drive) {
  uint32_t capacity;
  pcg_prnscr("BOOT: IDC dd");
  print_hex_digit(drive);
  pcg_prnscr(" ");
  if (idc_identify(drive, &capacity) != 0u) {
    pcg_prnlnscr("no media");
    return;
  }
  uint32_t result = idc_read(drive, 0u, 1u, (void *)(uintptr_t)BOOT_ENTRY);
  if (result != 0u) {
    print_colored(COLOR_NG, "read error ");
    print_hex2(result == 2u ? idc_error_code() : 0xFFu);
    newline();
    clear_boot_sector();
    return;
  }
  if (ram_byte(BOOT_ENTRY + BOOT_SIGNATURE_OFFSET) != 0x55u ||
      ram_byte(BOOT_ENTRY + BOOT_SIGNATURE_OFFSET + 1u) != 0xAAu) {
    pcg_prnlnscr("no boot signature");
    clear_boot_sector();
    return;
  }
  print_colored(COLOR_OK, "boot sector OK");
  newline();
  boot(drive);
}

int main(void) {
  pcg_initPCG();
  pcg_col40();
  post();
  hold_post();

  pcg_prnscr("BOOT: RAM 0x00000000 ");
  if (ram_word(BOOT_ENTRY) != 0u) {
    print_colored(COLOR_OK, "program found");
    newline();
    boot(BOOT_DEVICE_RAM);
  }
  pcg_prnlnscr("empty");

  pcg_prnlnscr("BOOT: ROM not implemented, skipped");

  for (uint32_t drive = 0; drive < IDC_DRIVES; drive++)
    try_idc(drive);

  print_colored(COLOR_NG, "BOOT: no bootable device");
  newline();
  irqc_disable(0xFFFFu);
  irqc_ack(0xFFFFu);
  for (;;)
    cpt32_halt();
}
