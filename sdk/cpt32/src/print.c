#include <cpt32/mmio.h>
#include <cpt32/print.h>
#define PRINT_UART_ADDR 0x80040000u
void print_putc(uint32_t c) { CPT32_MMIO8(PRINT_UART_ADDR) = (uint8_t)c; }
void print_prn(const char *s) {
  while (*s)
    print_putc((uint8_t)*s++);
}
void print_prnln(const char *s) {
  print_prn(s);
  print_putc(0x0d);
  print_putc(0x0a);
}
