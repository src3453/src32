# libfnd の C 実装を呼ぶ。tools/solc/build_foundation.py でビルドする。
# FND_LIB_ENTRY はビルダーが実際の C dispatcher のアドレスとして定義する。
!abi_compat "Foundation" 1
!include "fnd.sol"

!const FND_END 1
!const FND_E_NOT_DIR -64
!const FND_E_IS_DIR -65
!const FND_E_NOT_EMPTY -66
!const FND_E_BAD_PATH -67
!const FND_E_NAME_TOO_LONG -68
!const FND_E_NO_SPACE -69
!const FND_E_CROSS_DEVICE -70
!const FND_E_INTERRUPTED -71
!const FND_E_NEEDS_FSCK -72
!const FND_OPEN_READ 1
!const FND_OPEN_WRITE 2
!const FND_OPEN_CREATE 4
!const FND_OPEN_EXCL 8
!const FND_OPEN_TRUNC 16
!const FND_OPEN_APPEND 32
!const FND_OPEN_EXEC 64
!const FND_MODE_DEFAULT 0xFFFFFFFF
!const FND_CON_LINE 0
!const FND_CON_RAW 1
!const FND_STAT_SIZE 64
!const FND_STAT_TYPE 0
!const FND_STAT_MODE 4
!const FND_STAT_UID 8
!const FND_STAT_FLAGS 12
!const FND_STAT_SIZE_HI 16
!const FND_STAT_SIZE_LO 20
!const FND_STAT_CREATED_SEC_HI 24
!const FND_STAT_CREATED_SEC_LO 28
!const FND_STAT_CREATED_MS 32
!const FND_STAT_MODIFIED_SEC_HI 36
!const FND_STAT_MODIFIED_SEC_LO 40
!const FND_STAT_MODIFIED_MS 44
!const FND_STAT_CHANGED_SEC_HI 48
!const FND_STAT_CHANGED_SEC_LO 52
!const FND_STAT_CHANGED_MS 56
!const FND_DIRENT_HEAD 72
!const FND_DIRENT_MAX 1096
!const FND_EXEC_SIZE 28
!const FND_EXEC_ARGC 0
!const FND_EXEC_ARGV 4
!const FND_EXEC_CWD 8
!const FND_EXEC_STDIN 12
!const FND_EXEC_STDOUT 16
!const FND_EXEC_STDERR 20
!const FND_EXEC_FLAGS 24
!const FND_KEY_SIZE 12
!const FND_KEY_CODE 0
!const FND_KEY_CHAR 4
!const FND_KEY_MODS 8

# 内部: sol のフレームから C の stack ABI へコピーする。
# C が壊せる sol の R26/R27 と module origin R30 を保存し、SP を8境界にする。
fn fnd_lib_call (target op a b c d) :
    local r
!asm
    ADDI R28, R28, -64
    LDIH R13, 0xFFFF
    LDIL R13, 0xFFF8
    AND R28, R28, R13
    ST R26, [R28 + 24]
    ST R27, [R28 + 28]
    ST R30, [R28 + 32]
    LD R12, [R26 + 12]
    LD R13, [R26 + 16]
    ST R13, [R28 + 0]
    LD R13, [R26 + 20]
    ST R13, [R28 + 4]
    LD R13, [R26 + 24]
    ST R13, [R28 + 8]
    LD R13, [R26 + 28]
    ST R13, [R28 + 12]
    LD R13, [R26 + 32]
    ST R13, [R28 + 16]
    JALR R12
    LD R26, [R28 + 24]
    LD R27, [R28 + 28]
    LD R30, [R28 + 32]
    ADDI R28, R26, 0
    ST R1, [R26 + 8]
!end
    r ret
;

# 初期化を一度呼んでから他の高水準 API を使う。引数と戻り値は C と同順。
fn fnd_init () : FND_LIB_ENTRY 0 0 0 0 0 fnd_lib_call ret ;
fn fnd_argc () : FND_LIB_ENTRY 1 0 0 0 0 fnd_lib_call ret ;
fn fnd_argv (i) : FND_LIB_ENTRY 2 i 0 0 0 fnd_lib_call ret ;
fn fnd_std_in () : FND_LIB_ENTRY 3 0 0 0 0 fnd_lib_call ret ;
fn fnd_std_out () : FND_LIB_ENTRY 4 0 0 0 0 fnd_lib_call ret ;
fn fnd_std_err () : FND_LIB_ENTRY 5 0 0 0 0 fnd_lib_call ret ;
fn fnd_console () : FND_LIB_ENTRY 6 0 0 0 0 fnd_lib_call ret ;
fn fnd_fs () : FND_LIB_ENTRY 7 0 0 0 0 fnd_lib_call ret ;
fn fnd_exit (code) : code fnd_proc_exit retn ;

# 出力引数は4-byte境界の書き込み可能なポインタ。status と出力値を区別する。
fn fnd_file_open (path flags mode out) : FND_LIB_ENTRY 8 path flags mode out fnd_lib_call ret ;
fn fnd_read (h buf len got) : FND_LIB_ENTRY 9 h buf len got fnd_lib_call ret ;
fn fnd_write (h buf len put) : FND_LIB_ENTRY 10 h buf len put fnd_lib_call ret ;
fn fnd_close (h) : FND_LIB_ENTRY 11 h 0 0 0 fnd_lib_call ret ;
fn fnd_stream_stat (h st) : FND_LIB_ENTRY 12 h st 0 0 fnd_lib_call ret ;
fn fnd_path_stat (path st) : FND_LIB_ENTRY 13 path st 0 0 fnd_lib_call ret ;
fn fnd_cwd_set (path) : FND_LIB_ENTRY 14 path 0 0 0 fnd_lib_call ret ;
fn fnd_cwd_get (buf cap len) : FND_LIB_ENTRY 15 buf cap len 0 fnd_lib_call ret ;
fn fnd_con_write (text) : FND_LIB_ENTRY 16 text 0 0 0 fnd_lib_call ret ;
fn fnd_con_getkey (key) : FND_LIB_ENTRY 17 key 0 0 0 fnd_lib_call ret ;
fn fnd_con_pollkey (key) : FND_LIB_ENTRY 18 key 0 0 0 fnd_lib_call ret ;
fn fnd_con_set_mode (mode) : FND_LIB_ENTRY 19 mode 0 0 0 fnd_lib_call ret ;
fn fnd_con_clear () : FND_LIB_ENTRY 20 0 0 0 0 fnd_lib_call ret ;
fn fnd_con_size (cols rows) : FND_LIB_ENTRY 21 cols rows 0 0 fnd_lib_call ret ;
fn fnd_proc_exec (path ex proc) : FND_LIB_ENTRY 22 path ex proc 0 fnd_lib_call ret ;
# wait は PROCESS ハンドルを閉じる。さらに close しない。
fn fnd_proc_wait (proc code) : FND_LIB_ENTRY 23 proc code 0 0 fnd_lib_call ret ;
# fnd_proc_kill は fnd.sol の同名 syscall ラッパーをそのまま使う。
fn fnd_mem_alloc (size) : FND_LIB_ENTRY 25 size 0 0 0 fnd_lib_call ret ;
fn fnd_mem_free (p) : FND_LIB_ENTRY 26 p 0 0 0 fnd_lib_call drop retn ;
fn fnd_strerror (status) : FND_LIB_ENTRY 27 status 0 0 0 fnd_lib_call ret ;

# DIR_OPEN は実際の fs に送る。bootfs は未実装なので -13 を返す。
# DIR_NEXT は生 wire records を返す（高水準 fnd_dir_next は未提供）。
fn fnd_dir_open (path out) : FND_LIB_ENTRY 28 path out 0 0 fnd_lib_call ret ;
fn fnd_dir_read (h buf cap got) : FND_LIB_ENTRY 29 h buf cap got fnd_lib_call ret ;
fn fnd_dir_rewind (h) : FND_LIB_ENTRY 30 h 0 0 0 fnd_lib_call ret ;
