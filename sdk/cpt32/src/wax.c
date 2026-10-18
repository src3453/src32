#include <cpt32/irqc.h>
#include <cpt32/wax.h>
#include <stddef.h>
#include <stdint.h>

static uint32_t wax_state_entry;
static uint32_t wax_state_table_offset;
static uint32_t wax_state_block_count;

uint32_t wax_be16(const void *buffer, uint32_t offset) {
  const uint8_t *p = (const uint8_t *)buffer;
  if (p == NULL)
    return 0u;
  return ((uint32_t)p[offset] << 8) | p[offset + 1u];
}

uint32_t wax_be32(const void *buffer, uint32_t offset) {
  const uint8_t *p = (const uint8_t *)buffer;
  if (p == NULL)
    return 0u;
  return ((uint32_t)p[offset] << 24) | ((uint32_t)p[offset + 1u] << 16) |
         ((uint32_t)p[offset + 2u] << 8) | p[offset + 3u];
}

bool wax_ult(uint32_t a, uint32_t b) { return a < b; }

uint32_t wax_mark_span(uint32_t start, uint32_t size) {
  uint8_t *map = (uint8_t *)(uintptr_t)WAX_STAGE;
  uint32_t i;
  if (start > WAX_MAX_FILE || size > WAX_MAX_FILE - start)
    return WAX_ERR_RANGE;
  for (i = 0u; i < size; ++i) {
    uint32_t pos = start + i;
    uint32_t byte_index = pos >> 3;
    uint8_t mask = (uint8_t)(1u << (pos & 7u));
    if ((map[byte_index] & mask) != 0u)
      return WAX_ERR_OVERLAP;
    map[byte_index] |= mask;
  }
  return WAX_OK;
}

uint32_t wax_validate(const void *buffer, uint32_t file_size) {
  const uint8_t *data = (const uint8_t *)buffer;
  uintptr_t address;
  uint32_t flags;
  uint32_t base;
  uint32_t entry_offset;
  uint32_t table_offset;
  uint32_t table_end;
  uint32_t block_count;
  uint32_t debug_offset;
  uint32_t debug_size;
  uint32_t index;
  uint32_t previous_last = 0u;
  uint32_t entry;
  uint32_t found = 0u;
  uint8_t *span_map;

  if (file_size < 64u || file_size > WAX_MAX_FILE)
    return WAX_ERR_HEADER;
  if (buffer == NULL)
    return WAX_ERR_BUFFER;
  address = (uintptr_t)buffer;
  if (address > UINT32_MAX - (file_size - 1u))
    return WAX_ERR_BUFFER;
  if (address >= WAX_RESERVED_END || address + file_size - 1u >= WAX_STAGE)
    return WAX_ERR_BUFFER;

  if (wax_be32(data, 0u) != 0x57415800u || wax_be16(data, 4u) != 2u ||
      wax_be16(data, 6u) != 1u || wax_be16(data, 8u) != 1u)
    return WAX_ERR_HEADER;
  flags = wax_be16(data, 10u);
  if ((flags & 0xFFFEu) != 0u || wax_be32(data, 12u) != 64u ||
      wax_be32(data, 44u) != file_size)
    return WAX_ERR_HEADER;
  for (index = 48u; index < 64u; ++index) {
    if (data[index] != 0u)
      return WAX_ERR_HEADER;
  }

  base = wax_be32(data, 16u);
  entry_offset = wax_be32(data, 20u);
  table_offset = wax_be32(data, 24u);
  block_count = wax_be32(data, 28u);
  if (wax_be32(data, 32u) != 12u)
    return WAX_ERR_LAYOUT;
  debug_offset = wax_be32(data, 36u);
  debug_size = wax_be32(data, 40u);
  if (block_count == 0u || table_offset < 64u || (table_offset & 3u) != 0u ||
      table_offset > file_size ||
      block_count > (file_size - table_offset) / 12u)
    return WAX_ERR_LAYOUT;
  table_end = table_offset + block_count * 12u;

  if (debug_size != 0u) {
    if ((flags & 1u) == 0u)
      return WAX_ERR_LAYOUT;
    if ((debug_offset & 3u) != 0u || debug_offset < table_end)
      return WAX_ERR_LAYOUT;
    if (debug_offset > file_size || debug_size > file_size - debug_offset)
      return WAX_ERR_RANGE;
  } else if ((flags & 1u) != 0u || debug_offset != 0u) {
    return WAX_ERR_LAYOUT;
  }

  if (wax_be32(data, table_offset) != base)
    return WAX_ERR_LAYOUT;
  span_map = (uint8_t *)(uintptr_t)WAX_STAGE;
  for (index = 0u; index < WAX_SPAN_MAP_BYTES; ++index)
    span_map[index] = 0u;

  if (debug_size != 0u) {
    uint32_t status = wax_mark_span(debug_offset, debug_size);
    if (status != WAX_OK)
      return status;
  }

  for (index = 0u; index < block_count; ++index) {
    uint32_t record_offset = table_offset + index * 12u;
    uint32_t load_address = wax_be32(data, record_offset);
    uint32_t file_offset = wax_be32(data, record_offset + 4u);
    uint32_t block_size = wax_be32(data, record_offset + 8u);
    uint32_t load_last;
    uint32_t status;
    if (block_size == 0u)
      return WAX_ERR_RANGE;
    if (block_size - 1u > UINT32_MAX - load_address)
      return WAX_ERR_RANGE;
    load_last = load_address + block_size - 1u;
    if (file_offset < table_end || (file_offset & 3u) != 0u)
      return WAX_ERR_RANGE;
    if (file_offset > file_size || block_size > file_size - file_offset)
      return WAX_ERR_RANGE;
    if (load_address < WAX_RESERVED_END && load_last >= WAX_STAGE)
      return WAX_ERR_RESERVED;
    if (index != 0u && load_address <= previous_last)
      return WAX_ERR_LAYOUT;
    previous_last = load_last;
    status = wax_mark_span(file_offset, block_size);
    if (status != WAX_OK)
      return status;
  }

  if (entry_offset > UINT32_MAX - base)
    return WAX_ERR_ENTRY;
  entry = base + entry_offset;
  for (index = 0u; index < block_count; ++index) {
    uint32_t record_offset = table_offset + index * 12u;
    uint32_t load_address = wax_be32(data, record_offset);
    uint32_t block_size = wax_be32(data, record_offset + 8u);
    uint32_t load_last = load_address + block_size - 1u;
    if (entry >= load_address && entry <= load_last)
      found = 1u;
  }
  if (found == 0u)
    return WAX_ERR_ENTRY;

  wax_state_entry = entry;
  wax_state_table_offset = table_offset;
  wax_state_block_count = block_count;
  return WAX_OK;
}

/* Raw SRC32 trampoline source (all registers and absolute addresses are fixed):
 *   LD R1, [R28 + 0]       ; staged image base, args at incoming SP+0..12
 *   LD R2, [R28 + 4]       ; block table offset
 *   LD R3, [R28 + 8]       ; block count
 *   LD R4, [R28 + 12]      ; entry address
 *   ADD R2, R1, R2
 * block: BEQ R3, R0, launch; LD R5,[R2]; LD R6,[R2+4]; LD R7,[R2+8]
 *   ADD R6,R1,R6
 * copy: BEQ R7,R0,next; LDB R8,[R6]; STB [R5],R8
 *   ADDI R5,R5,1; ADDI R6,R6,1; ADDI R7,R7,-1; JMP copy
 * next: ADDI R2,R2,12; ADDI R3,R3,-1; JMP block
 * launch: JR R4
 * Encoded by tools/asm/asm.py; readable source is wax_trampoline.asm. It is
 * copied before IRQs are disabled or blocks loaded. */
static const uint32_t wax_trampoline_words[] = {
    0x043C0000u, 0x045C0004u, 0x047C0008u, 0x049C000Cu, 0x0C411000u,
    0x1C030038u, 0x04A20000u, 0x04C20004u, 0x04E20008u, 0x0CC13000u,
    0x1C070018u, 0x4D060000u, 0x55050000u, 0x10A50001u, 0x10C60001u,
    0x10E7FFFFu, 0x2400FFE4u, 0x1042000Cu, 0x1063FFFFu, 0x2400FFC4u,
    0x2C800000u};

typedef void (*wax_trampoline_fn)(const void *, uint32_t, uint32_t, uint32_t);

void wax_transfer(const void *staged_buffer, uint32_t table_offset,
                  uint32_t block_count, uint32_t entry) {
  volatile uint32_t *destination =
      (volatile uint32_t *)(uintptr_t)WAX_TRAMPOLINE;
  uint32_t i;
  for (i = 0u;
       i < sizeof(wax_trampoline_words) / sizeof(wax_trampoline_words[0]); ++i)
    destination[i] = wax_trampoline_words[i];
  irqc_disable(0xFFFFu);
  irqc_ack(0xFFFFu);
  ((wax_trampoline_fn)(uintptr_t)WAX_TRAMPOLINE)(staged_buffer, table_offset,
                                                 block_count, entry);
}

uint32_t wax_load(const void *buffer, uint32_t file_size) {
  const uint8_t *source = (const uint8_t *)buffer;
  uint8_t *stage = (uint8_t *)(uintptr_t)WAX_STAGE;
  uint32_t i;
  uint32_t status = wax_validate(buffer, file_size);
  if (status != WAX_OK)
    return status;
  for (i = 0u; i < file_size; ++i)
    stage[i] = source[i];
  wax_transfer(stage, wax_state_table_offset, wax_state_block_count,
               wax_state_entry);
  return WAX_ERR_BUFFER;
}

uint32_t wax_get_entry(void) { return wax_state_entry; }
uint32_t wax_get_table_offset(void) { return wax_state_table_offset; }
uint32_t wax_get_block_count(void) { return wax_state_block_count; }
