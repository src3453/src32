# DSFS (Dead Simple File System) guest library (doc/spec_DSFS.md).
#
# Include sol/cpt32/idc.sol in the same program; every operation talks
# to the drive through that driver (DMAC channel 0, polling). Mount
# state is cached in globals, so mount once per volume and call
# dsfs_set_time before writes if the guest knows the clock (the default
# timestamp written into new entries is 0).
#
# Buffer contracts (main RAM):
# - mount/dir buffer: at least 8192 bytes (the 16-sector directory);
#   the same buffer may be passed to dsfs_mount and dsfs_dir_load.
# - bitmap buffer: at least dsfs_bitmap_blocks*512 bytes, i.e. 512
#   bytes for volumes up to 32768 blocks (16 MiB).
# - file buffers: at least the file size rounded up to whole sectors.
#
# Error codes (plain integers, not sol truth values):
!namespace dsfs -
!const DSFS_OK 0
!const DSFS_ERR_IO 1       # IDC/DMAC transfer failed
!const DSFS_ERR_MAGIC 2    # superblock magic is not "DSFS"
!const DSFS_ERR_VERSION 3  # version != 1 or flags != 0
!const DSFS_ERR_LAYOUT 4   # superblock layout does not match spec §2
!const DSFS_ERR_NOT_FOUND 5
!const DSFS_ERR_FULL 6     # 256 entries used or no contiguous space
!const DSFS_ERR_ARGS 7     # bad argument, or volume not mounted
!const DSFS_DIR_BLOCKS 16
!const DSFS_ENTRIES 256
!const DSFS_ENTRY_SIZE 32
!const DSFS_NAME_BYTES 11

# Cached superblock fields (set by dsfs_mount; read-only afterwards).
!var dsfs_mounted 0
!var dsfs_slot 0
!var dsfs_total 0
!var dsfs_bitmap_start 0
!var dsfs_bitmap_blocks 0
!var dsfs_dir_start 0
!var dsfs_data_start 0
# Timestamp written into created/modified of new entries.
!var dsfs_time 0
# Base of the loaded bitmap buffer (set by dsfs_bitmap_load).
!var dsfs_bitmap_buf 0

# Set the timestamp (Unix seconds) stored by writes; 0 by default.
fn dsfs_set_time (t) :
    t >dsfs_time
    retn
;

# Mount the volume in `slot`: read LBA 1 into `buffer` (>= 512 bytes),
# validate magic/version/layout (spec §7) and cache the geometry.
fn dsfs_mount (slot buffer) :
    0 >dsfs_mounted
    slot >dsfs_slot
    slot 1 1 buffer idc.idc_read 0 neq if DSFS_ERR_IO ret end
    buffer ld 0x44534653 neq if DSFS_ERR_MAGIC ret end
    buffer 4 add ldh 1 neq if DSFS_ERR_VERSION ret end
    buffer 6 add ldh 0 neq if DSFS_ERR_VERSION ret end
    buffer 8 add ld >dsfs_total
    buffer 12 add ld >dsfs_bitmap_start
    buffer 16 add ld >dsfs_bitmap_blocks
    buffer 20 add ld >dsfs_dir_start
    buffer 28 add ld >dsfs_data_start
    dsfs_bitmap_start 2 neq if DSFS_ERR_LAYOUT ret end
    dsfs_total 4095 add 12 shr dsfs_bitmap_blocks neq if DSFS_ERR_LAYOUT ret end
    dsfs_dir_start dsfs_bitmap_start dsfs_bitmap_blocks add neq if DSFS_ERR_LAYOUT ret end
    buffer 24 add ld 16 neq if DSFS_ERR_LAYOUT ret end
    dsfs_data_start dsfs_dir_start 16 add neq if DSFS_ERR_LAYOUT ret end
    dsfs_data_start dsfs_total gt if DSFS_ERR_LAYOUT ret end
    1 >dsfs_mounted
    DSFS_OK ret
;

# Load the whole directory (16 sectors) into `buffer` (>= 8192 bytes).
fn dsfs_dir_load (buffer) :
    dsfs_mounted 0 eq if DSFS_ERR_ARGS ret end
    dsfs_slot dsfs_dir_start 16 buffer idc.idc_read 0 eq if DSFS_OK ret end
    DSFS_ERR_IO ret
;

# Write the single directory sector that holds entry `index` back to
# the volume; `dirbuf` must be a freshly loaded directory buffer.
fn dsfs_dir_save_sector (dirbuf index) :
    dsfs_slot dsfs_dir_start index 4 shr add 1 dirbuf index 4 shr 9 shl add idc.idc_write 0 eq if DSFS_OK ret end
    DSFS_ERR_IO ret
;

# Load the allocation bitmap into `buffer` and remember its base for
# dsfs_bm_get / dsfs_bm_set / dsfs_bm_clear.
fn dsfs_bitmap_load (buffer) :
    dsfs_mounted 0 eq if DSFS_ERR_ARGS ret end
    buffer >dsfs_bitmap_buf
    dsfs_slot dsfs_bitmap_start dsfs_bitmap_blocks buffer idc.idc_read 0 eq if DSFS_OK ret end
    DSFS_ERR_IO ret
;

# Write the loaded bitmap back to the volume.
fn dsfs_bitmap_save () :
    dsfs_slot dsfs_bitmap_start dsfs_bitmap_blocks dsfs_bitmap_buf idc.idc_write 0 eq if DSFS_OK ret end
    DSFS_ERR_IO ret
;

# Bitmap bit of `block`: 1 = used, 0 = free. Needs dsfs_bitmap_load.
fn dsfs_bm_get (block) :
    dsfs_bitmap_buf block 3 shr add ldb 128 block 7 and shr and 0 eq if 0 ret end
    1 ret
;

# Set / clear one bitmap bit in the loaded buffer (no disk I/O).
fn dsfs_bm_set (block) :
    local addr 0
    dsfs_bitmap_buf block 3 shr add >addr
    addr ldb 128 block 7 and shr or addr stb
    retn
;

fn dsfs_bm_clear (block) :
    local addr 0
    local mask 0
    dsfs_bitmap_buf block 3 shr add >addr
    128 block 7 and shr >mask
    addr ldb mask not and addr stb
    retn
;

# Address of directory entry `index` inside a loaded directory buffer.
fn dsfs_entry_addr (buffer index) :
    buffer index 5 shl add ret
;

fn dsfs_entry_status (addr) :
    addr ldb ret
;

fn dsfs_entry_start (addr) :
    addr 16 add ld ret
;

fn dsfs_entry_size (addr) :
    addr 20 add ld ret
;

fn dsfs_entry_created (addr) :
    addr 24 add ld ret
;

fn dsfs_entry_modified (addr) :
    addr 28 add ld ret
;

# Compare the 11 on-disk name bytes at `a` and `b`: 0 means equal
# (sol truth), nonzero means different. Names are e.g. string literals
# like "FILE TXT" (8.3, space padded, exactly 11 bytes).
fn dsfs_name_eq (a b) :
    local acc 0
    local i 0
    while
        i 11 lt
        if
            a i add ldb b i add ldb eq acc or >acc
            i 1 add >i
            0
        else
            1
        end
    end
    acc ret
;

# Find `name` in a loaded directory buffer: returns the entry index
# 0..255, or -1 when the name is not present.
fn dsfs_find (buffer name) :
    local addr 0
    local i 0
    while
        i 256 lt
        if
            buffer i 5 shl add >addr
            addr ldb 1 eq if
                addr 1 add name dsfs_name_eq 0 eq if
                    i ret
                end
            end
            i 1 add >i
            0
        else
            1
        end
    end
    -1 ret
;

# Count used entries in a loaded directory buffer (0..256).
fn dsfs_count (buffer) :
    local n 0
    local i 0
    while
        i 256 lt
        if
            buffer i 5 shl add ldb 1 eq if
                n 1 add >n
            end
            i 1 add >i
            0
        else
            1
        end
    end
    n ret
;

# Read the whole file `name` into `dest` (>= its size in bytes). The
# directory is (re)loaded into `dirbuf` first. 0-byte files transfer
# nothing and succeed.
fn dsfs_read_file (dirbuf name dest) :
    local idx 0
    local addr 0
    local size 0
    local sectors 0
    dirbuf dsfs_dir_load 0 neq if DSFS_ERR_IO ret end
    dirbuf name dsfs_find >idx
    idx 0 lt if DSFS_ERR_NOT_FOUND ret end
    dirbuf idx 5 shl add >addr
    addr 20 add ld >size
    size 0 eq if DSFS_OK ret end
    size 511 add 9 shr >sectors
    dsfs_slot addr 16 add ld sectors dest idc.idc_read 0 eq if DSFS_OK ret end
    DSFS_ERR_IO ret
;

# Create or replace `name` with `bytes` of data from `src`. The tail of
# the last sector inside `src` (bytes `bytes` .. `sectors*512-1`) is
# zero-filled in place by this call, so `src` must hold a full
# `sectors*512` bytes. `bytes` may be 0 (empty file, no blocks). The
# spec §8 update order is followed: new bitmap bits, data, directory
# entry, then blocks the replaced region no longer covers are released.
fn dsfs_write_file (dirbuf bitmapbuf name src bytes) :
    local idx 0
    local addr 0
    local size 0
    local sectors 0
    local old_start 0
    local old_blocks 0
    local run_start -1
    local cand 0
    local ok 0
    local j 0
    local k 0
    bytes sgn 0 neq if DSFS_ERR_ARGS ret end
    bytes 511 add 9 shr >sectors
    sectors 8388608 ge if DSFS_ERR_ARGS ret end
    dirbuf dsfs_dir_load 0 neq if DSFS_ERR_IO ret end
    dirbuf name dsfs_find >idx
    idx 0 lt if
        # No existing entry: take the first free slot.
        0 >k
        -1 >idx
        while
            k 256 lt idx 0 lt or
            if
                dirbuf k 5 shl add ldb 0 eq if
                    k >idx
                end
                k 1 add >k
                0
            else
                1
            end
        end
        idx 0 lt if DSFS_ERR_FULL ret end
    end
    dirbuf idx 5 shl add >addr
    addr 16 add ld >old_start
    addr 20 add ld >size
    size 0 eq if
        0 >old_blocks
    else
        size 511 add 9 shr >old_blocks
    end
    # Reuse the current region when it already covers the new size.
    sectors old_blocks le old_blocks 0 gt or if
        old_start >run_start
    end
    run_start 0 lt if
        bitmapbuf dsfs_bitmap_load 0 neq if DSFS_ERR_IO ret end
        dsfs_data_start >cand
        while
            run_start 0 lt cand dsfs_total sectors sub le or
            if
                1 >ok
                0 >j
                while
                    j sectors lt ok 0 neq or
                    if
                        cand j add dsfs_bm_get 0 neq if
                            0 >ok
                        end
                        j 1 add >j
                        0
                    else
                        1
                    end
                end
                ok 1 eq if
                    cand >run_start
                end
                cand 1 add >cand
                0
            else
                1
            end
        end
        run_start 0 lt if DSFS_ERR_FULL ret end
        0 >j
        while
            j sectors lt
            if
                run_start j add dsfs_bm_set
                j 1 add >j
                0
            else
                1
            end
        end
        dsfs_bitmap_save 0 neq if DSFS_ERR_IO ret end
    end
    # Zero the tail of the final sector, then transfer the data.
    bytes >k
    while
        k sectors 9 shl lt
        if
            0 src k add stb
            k 1 add >k
            0
        else
            1
        end
    end
    sectors 0 gt if
        dsfs_slot run_start sectors src idc.idc_write 0 neq if DSFS_ERR_IO ret end
    end
    # Fill and persist the directory entry (start is 0 for empty files).
    bytes 0 eq if
        0 >run_start
    end
    1 addr stb
    0 >k
    while
        k 11 lt
        if
            name k add ldb addr 1 add k add stb
            k 1 add >k
            0
        else
            1
        end
    end
    0 >k
    while
        k 4 lt
        if
            0 addr 12 add k add stb
            k 1 add >k
            0
        else
            1
        end
    end
    run_start addr 16 add st
    bytes addr 20 add st
    dsfs_time addr 24 add st
    dsfs_time addr 28 add st
    dirbuf idx 4 shr dsfs_dir_save_sector 0 neq if DSFS_ERR_IO ret end
    # Release old blocks the new region does not cover (moved or shrank).
    old_blocks 0 gt old_blocks sectors gt run_start old_start neq and or if
        bitmapbuf dsfs_bitmap_load 0 neq if DSFS_ERR_IO ret end
        0 >k
        while
            k old_blocks lt
            if
                run_start old_start eq k sectors lt or 1 xor if
                    old_start k add dsfs_bm_clear
                end
                k 1 add >k
                0
            else
                1
            end
        end
        dsfs_bitmap_save 0 neq if DSFS_ERR_IO ret end
    end
    DSFS_OK ret
;

# Delete `name`: the entry is cleared before its bitmap bits, as the
# spec §8 ordering requires.
fn dsfs_delete_file (dirbuf bitmapbuf name) :
    local idx 0
    local addr 0
    local size 0
    local start0 0
    local k 0
    dirbuf dsfs_dir_load 0 neq if DSFS_ERR_IO ret end
    dirbuf name dsfs_find >idx
    idx 0 lt if DSFS_ERR_NOT_FOUND ret end
    dirbuf idx 5 shl add >addr
    addr 16 add ld >start0
    addr 20 add ld >size
    0 >k
    while
        k 32 lt
        if
            0 addr k add stb
            k 1 add >k
            0
        else
            1
        end
    end
    dirbuf idx 4 shr dsfs_dir_save_sector 0 neq if DSFS_ERR_IO ret end
    size 0 eq if DSFS_OK ret end
    bitmapbuf dsfs_bitmap_load 0 neq if DSFS_ERR_IO ret end
    size 511 add 9 shr >k
    0 >idx
    while
        idx k lt
        if
            start0 idx add dsfs_bm_clear
            idx 1 add >idx
            0
        else
            1
        end
    end
    dsfs_bitmap_save 0 neq if DSFS_ERR_IO ret end
    DSFS_OK ret
;
!end
