#include <cpt32/dsfs.h>
#include <cpt32/idc.h>
#include <stdint.h>

static uint8_t directory[DSFS_DIR_BLOCKS * DSFS_SECTOR_SIZE];
static uint8_t bitmap[DSFS_SECTOR_SIZE];
static uint8_t file_source[1536];
static uint8_t file_result[1536];
static const uint8_t name[DSFS_NAME_BYTES] = {'S', 'M', 'O', 'K', 'E', ' ',
                                              ' ', ' ', 'T', 'X', 'T'};

int main(void) {
  uint32_t i;
  uint32_t capacity;
  uint32_t first_start;
  int32_t index;
  uint8_t *entry;
  if (idc_identify(0u, &capacity) != 0u || capacity != 64u)
    return 101;
  if (dsfs_mount(0u, directory) != DSFS_OK)
    return 102;
  if (dsfs_total_blocks() != 64u || dsfs_bitmap_blocks() != 1u ||
      dsfs_data_start() != 19u)
    return 103;
  if (dsfs_dir_load(directory) != DSFS_OK || dsfs_count(directory) != 0u)
    return 104;

  dsfs_set_time(1700000000u);
  for (i = 0u; i < 600u; ++i)
    file_source[i] = (uint8_t)(i * 7u + 3u);
  if (dsfs_write_file(directory, bitmap, name, file_source, 600u) != DSFS_OK)
    return 105;
  for (i = 600u; i < 1024u; ++i) {
    if (file_source[i] != 0u)
      return 106;
  }
  if (dsfs_read_file(directory, name, file_result) != DSFS_OK)
    return 107;
  if (file_result[0] != 3u || file_result[599] != (uint8_t)(599u * 7u + 3u) ||
      file_result[600] != 0u || file_result[1023] != 0u)
    return 108;
  index = dsfs_find(directory, name);
  if (index != 0)
    return 109;
  entry = dsfs_entry_addr(directory, (uint32_t)index);
  if (dsfs_entry_size(entry) != 600u ||
      dsfs_entry_created(entry) != 1700000000u ||
      dsfs_entry_modified(entry) != 1700000000u)
    return 110;
  first_start = dsfs_entry_start(entry);
  if (first_start != 19u)
    return 111;

  dsfs_set_time(1700000123u);
  for (i = 0u; i < 1200u; ++i)
    file_source[i] = (uint8_t)(i ^ 0x5Au);
  if (dsfs_write_file(directory, bitmap, name, file_source, 1200u) != DSFS_OK)
    return 112;
  index = dsfs_find(directory, name);
  entry = dsfs_entry_addr(directory, (uint32_t)index);
  if (dsfs_entry_start(entry) != 21u || dsfs_entry_size(entry) != 1200u ||
      dsfs_entry_created(entry) != 1700000000u ||
      dsfs_entry_modified(entry) != 1700000123u)
    return 113;
  if (dsfs_bitmap_load(bitmap) != DSFS_OK || dsfs_bm_get(19u) != 0u ||
      dsfs_bm_get(20u) != 0u || dsfs_bm_get(21u) == 0u ||
      dsfs_bm_get(23u) == 0u)
    return 114;

  dsfs_set_time(1700000456u);
  if (dsfs_write_file(directory, bitmap, name, file_source, 100u) != DSFS_OK)
    return 115;
  if (dsfs_bitmap_load(bitmap) != DSFS_OK || dsfs_bm_get(21u) == 0u ||
      dsfs_bm_get(22u) != 0u || dsfs_bm_get(23u) != 0u)
    return 116;
  if (dsfs_delete_file(directory, bitmap, name) != DSFS_OK)
    return 117;
  if (dsfs_dir_load(directory) != DSFS_OK || dsfs_count(directory) != 0u ||
      dsfs_find(directory, name) != -1)
    return 118;
  if (dsfs_bitmap_load(bitmap) != DSFS_OK || dsfs_bm_get(21u) != 0u)
    return 119;
  return 42;
}
