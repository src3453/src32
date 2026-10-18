#include <cpt32/wax.h>
#include <stdint.h>

#define WAX_SMOKE_ENTRY 0x003F0000u
#define WAX_SMOKE_MARKER 0x003F0100u

static uint8_t image[96];
static uint8_t destructive[4192];

static void put_be16(uint8_t *p, uint16_t value) {
  p[0] = (uint8_t)(value >> 8);
  p[1] = (uint8_t)value;
}

static void put_be32(uint8_t *p, uint32_t value) {
  p[0] = (uint8_t)(value >> 24);
  p[1] = (uint8_t)(value >> 16);
  p[2] = (uint8_t)(value >> 8);
  p[3] = (uint8_t)value;
}

static void make_image(void) {
  static const uint32_t entry_code[] = {
      0x10A0002Au, /* ADDI R5, R0, 42 */
      0xF4C0003Fu, /* LDIH R6, 0x003F */
      0xF0C00100u, /* LDIL R6, 0x0100 */
      0x08A60000u, /* ST [R6 + 0], R5 */
      0x2FE00000u  /* JR R31 */
  };
  uint32_t i;
  image[0] = 'W';
  image[1] = 'A';
  image[2] = 'X';
  image[3] = 0u;
  put_be16(image + 4u, 2u);
  put_be16(image + 6u, 1u);
  put_be16(image + 8u, 1u);
  put_be16(image + 10u, 0u);
  put_be32(image + 12u, 64u);
  put_be32(image + 16u, WAX_SMOKE_ENTRY);
  put_be32(image + 20u, 0u);
  put_be32(image + 24u, 64u);
  put_be32(image + 28u, 1u);
  put_be32(image + 32u, 12u);
  put_be32(image + 36u, 0u);
  put_be32(image + 40u, 0u);
  put_be32(image + 44u, sizeof(image));
  put_be32(image + 64u, WAX_SMOKE_ENTRY);
  put_be32(image + 68u, 76u);
  put_be32(image + 72u, sizeof(entry_code));
  for (i = 0u; i < sizeof(entry_code) / sizeof(entry_code[0]); ++i)
    put_be32(image + 76u + i * 4u, entry_code[i]);
}

int main(void) {
  uint32_t status;
  make_image();
  if (wax_validate(image, sizeof(image)) != WAX_OK)
    return 101;
  if (wax_get_entry() != WAX_SMOKE_ENTRY || wax_get_table_offset() != 64u ||
      wax_get_block_count() != 1u)
    return 102;

  put_be16(image + 10u, 0x8000u);
  if (wax_validate(image, sizeof(image)) != WAX_ERR_HEADER)
    return 103;
  if (wax_get_entry() != WAX_SMOKE_ENTRY || wax_get_block_count() != 1u)
    return 104;
  put_be16(image + 10u, 0u);

  *(volatile uint32_t *)(uintptr_t)WAX_SMOKE_MARKER = 0u;
  status = wax_load(image, sizeof(image));
  if (status != WAX_ERR_BUFFER)
    return 105;
  if (*(volatile uint32_t *)(uintptr_t)WAX_SMOKE_MARKER != 42u)
    return 106;
  /* Final launch replaces live code at 0 and all of the current stack page.
   * It can succeed only if the copied trampoline is truly self-contained. */
  for (uint32_t i = 0; i < 64; ++i)
    destructive[i] = image[i];
  put_be32(destructive + 16, 0);
  put_be32(destructive + 28, 2);
  put_be32(destructive + 44, sizeof(destructive));
  put_be32(destructive + 64, 0);
  put_be32(destructive + 68, 88);
  put_be32(destructive + 72, 8);
  put_be32(destructive + 76, 0x000FF000u);
  put_be32(destructive + 80, 96);
  put_be32(destructive + 84, 4096);
  put_be32(destructive + 88, 0x1020002Au); /* ADDI R1, R0, 42 */
  put_be32(destructive + 92, 0xFC000000u); /* HALT */
  wax_load(destructive, sizeof(destructive));
  return 107; /* A successful destructive launch never returns. */
}
