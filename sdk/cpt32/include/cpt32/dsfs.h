#ifndef CPT32_DSFS_H
#define CPT32_DSFS_H
#include <stdbool.h>
#include <stdint.h>

#define DSFS_OK 0u
#define DSFS_ERR_IO 1u
#define DSFS_ERR_MAGIC 2u
#define DSFS_ERR_VERSION 3u
#define DSFS_ERR_LAYOUT 4u
#define DSFS_ERR_NOT_FOUND 5u
#define DSFS_ERR_FULL 6u
#define DSFS_ERR_ARGS 7u
#define DSFS_DIR_BLOCKS 16u
#define DSFS_ENTRIES 256u
#define DSFS_ENTRY_SIZE 32u
#define DSFS_NAME_BYTES 11u
#define DSFS_SECTOR_SIZE 512u

/* DSFS keeps one mounted volume and one remembered bitmap buffer globally.
 * Mount/dir buffers hold at least 8192 bytes; bitmap buffers hold
 * dsfs_bitmap_blocks()*512 bytes; file buffers hold the file's sector-rounded
 * size. Names are exactly 11 space-padded 8.3 bytes, without a dot or NUL. */
/* ファイル作成・更新日時に使うUNIX秒を設定します。 */
void dsfs_set_time(uint32_t unix_seconds);
/* 指定ドライブのスーパーブロックを検証し、単一ボリュームをマウントします。 */
uint32_t dsfs_mount(uint32_t slot, void *buffer);
/* マウント中のディレクトリ全体を8192 byteのバッファへ読み込みます。 */
uint32_t dsfs_dir_load(void *buffer);
/* 指定エントリを含む1セクタをディレクトリバッファから保存します。 */
uint32_t dsfs_dir_save_sector(void *dir_buffer, uint32_t index);
/* 割当ビットマップを読み込み、操作対象のバッファとして記憶します。 */
uint32_t dsfs_bitmap_load(void *buffer);
/* 記憶中の割当ビットマップをディスクへ保存します。 */
uint32_t dsfs_bitmap_save(void);
/* 記憶中のビットマップから指定ブロックの使用状態（0/1）を返します。 */
uint32_t dsfs_bm_get(uint32_t block);
/* 記憶中のビットマップで指定ブロックを使用中にします（保存は別途必要）。 */
void dsfs_bm_set(uint32_t block);
/* 記憶中のビットマップで指定ブロックを空きにします（保存は別途必要）。 */
void dsfs_bm_clear(uint32_t block);
/* ディレクトリバッファ内の指定エントリを返し、不正な引数ではNULLを返します。 */
uint8_t *dsfs_entry_addr(void *buffer, uint32_t index);
/* エントリの状態値（0=空き、1=使用中）を返します。 */
uint32_t dsfs_entry_status(const void *entry);
/* エントリに記録されたファイルの開始ブロックを返します。 */
uint32_t dsfs_entry_start(const void *entry);
/* エントリに記録されたファイルサイズをbyte単位で返します。 */
uint32_t dsfs_entry_size(const void *entry);
/* エントリの作成日時をUNIX秒で返します。 */
uint32_t dsfs_entry_created(const void *entry);
/* エントリの更新日時をUNIX秒で返します。 */
uint32_t dsfs_entry_modified(const void *entry);
/* 11 byteの名前を比較し、一致すればtrueを返します。 */
bool dsfs_name_eq(const void *a, const void *b);
/* ディレクトリバッファから名前を検索し、エントリ番号または未検出の-1を返します。 */
int32_t dsfs_find(const void *buffer, const void *name);
/* ディレクトリバッファ内の使用中エントリ数を返します。 */
uint32_t dsfs_count(const void *buffer);
/* 名前でファイルを探し、セクタ境界までのデータをdestへ読み込みます。 */
uint32_t dsfs_read_file(void *dir_buffer, const void *name, void *dest);
/* ファイルを作成・上書きし、srcの最終セクタの余白をゼロ埋めします。 */
uint32_t dsfs_write_file(void *dir_buffer, void *bitmap_buffer,
                         const void *name, void *src, uint32_t bytes);
/* 名前で指定したファイルを削除し、使用ブロックを解放します。 */
uint32_t dsfs_delete_file(void *dir_buffer, void *bitmap_buffer,
                          const void *name);

/* Read-only access to the singleton mount cache. bitmap_buffer is NULL until
 * dsfs_bitmap_load has been called successfully or attempted. */
/* 現在のマウントが成功していればtrueを返します。 */
bool dsfs_is_mounted(void);
/* 現在のマウント状態に記録されたドライブ番号を返します。 */
uint32_t dsfs_slot(void);
/* マウントしたボリュームの総ブロック数を返します。 */
uint32_t dsfs_total_blocks(void);
/* 割当ビットマップの開始ブロックを返します。 */
uint32_t dsfs_bitmap_start(void);
/* 割当ビットマップのブロック数を返します。 */
uint32_t dsfs_bitmap_blocks(void);
/* ディレクトリの開始ブロックを返します。 */
uint32_t dsfs_dir_start(void);
/* ファイルデータ領域の開始ブロックを返します。 */
uint32_t dsfs_data_start(void);
/* ファイル日時用に設定されたUNIX秒を返します。 */
uint32_t dsfs_time(void);
/* 記憶中のビットマップバッファを返します（未設定ならNULL）。 */
uint8_t *dsfs_bitmap_buffer(void);
#endif
