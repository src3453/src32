#ifndef CPT32_WAX_H
#define CPT32_WAX_H
#include <stdbool.h>
#include <stdint.h>

#define WAX_OK 0u
#define WAX_ERR_HEADER 1u
#define WAX_ERR_LAYOUT 2u
#define WAX_ERR_RANGE 3u
#define WAX_ERR_OVERLAP 4u
#define WAX_ERR_ENTRY 5u
#define WAX_ERR_RESERVED 6u
#define WAX_ERR_BUFFER 7u
#define WAX_MAX_FILE 65536u
#define WAX_STAGE 0x00FE0000u
#define WAX_RESERVED_END 0x01000000u
#define WAX_SPAN_MAP_BYTES 8192u
#define WAX_TRAMPOLINE 0x00FF0000u

/* File bytes must be readable in RAM below WAX_STAGE and may not overlap the
 * WAX loader's reserved 0x00FE0000..0x01000000 staging window. Validate only
 * consumes a temporary 8 KiB span map there. Load stages a validated image,
 * copies its sparse blocks, then jumps to entry; a transfer that returns is
 * reported as WAX_ERR_BUFFER. All functions use numeric WAX_* results. */
uint32_t wax_be16(const void *buffer, uint32_t offset);
uint32_t wax_be32(const void *buffer, uint32_t offset);
bool wax_ult(uint32_t a, uint32_t b);
uint32_t wax_mark_span(uint32_t start, uint32_t size);
uint32_t wax_validate(const void *buffer, uint32_t file_size);
uint32_t wax_load(const void *buffer, uint32_t file_size);
/* Transfers a staged WAX image; normally called by wax_load only. */
void wax_transfer(const void *staged_buffer, uint32_t table_offset,
                  uint32_t block_count, uint32_t entry);
uint32_t wax_get_entry(void);
uint32_t wax_get_table_offset(void);
uint32_t wax_get_block_count(void);
#endif
