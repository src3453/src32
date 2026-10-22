#ifndef FOUNDATION_API_H
#define FOUNDATION_API_H
#include <foundation/errors.h>

/* Numbers of the Foundation API (doc/spec_Foundation_API.md). Only the parts
 * that libfnd implements so far are listed. */

/* §3: API error codes, after the kernel's -1..-63. */
#define FND_E_NOT_DIR (-64)
#define FND_E_IS_DIR (-65)
#define FND_E_NOT_EMPTY (-66)
#define FND_E_BAD_PATH (-67)
#define FND_E_NAME_TOO_LONG (-68)
#define FND_E_NO_SPACE (-69)
#define FND_E_CROSS_DEVICE (-70)
#define FND_E_INTERRUPTED (-71)
#define FND_E_NEEDS_FSCK (-72)

/* §5: stream operations. */
#define FND_ST_READ 0x0001u
#define FND_ST_WRITE 0x0002u
#define FND_ST_SEEK 0x0003u
#define FND_ST_STAT 0x0004u
#define FND_ST_TRUNCATE 0x0005u
#define FND_ST_SYNC 0x0006u

#define FND_SEEK_SET 0u
#define FND_SEEK_CUR 1u
#define FND_SEEK_END 2u

/* §6.3: fs session operations. */
#define FND_FS_OPEN 0x0100u
#define FND_FS_STAT 0x0101u
#define FND_FS_DIR_OPEN 0x0102u
#define FND_FS_MKDIR 0x0103u
#define FND_FS_UNLINK 0x0104u
#define FND_FS_RMDIR 0x0105u
#define FND_FS_RENAME 0x0106u
#define FND_FS_CHMOD 0x0107u
#define FND_FS_CHOWN 0x0108u
#define FND_FS_SYNC 0x0109u
#define FND_FS_CWD_SET 0x010Au
#define FND_FS_CWD_GET 0x010Bu
#define FND_FS_SESSION_NEW 0x010Cu
#define FND_FS_VOLUME_INFO 0x010Du

/* §6.4: FS_OPEN flags. */
#define FND_OPEN_READ 0x01u
#define FND_OPEN_WRITE 0x02u
#define FND_OPEN_CREATE 0x04u
#define FND_OPEN_EXCL 0x08u
#define FND_OPEN_TRUNC 0x10u
#define FND_OPEN_APPEND 0x20u
#define FND_OPEN_EXEC 0x40u

#define FND_MODE_DEFAULT 0xFFFFFFFFu

/* §7: directory operations. */
#define FND_DIR_NEXT 0x0200u
#define FND_DIR_REWIND 0x0201u

/* §9: device services. */
#define FND_DEV_OPEN 0x0300u

/* §10.1 fnd_stat (64 bytes), §10.2 directory entries, §10.3 fnd_volinfo. */
#define FND_STAT_BYTES 64u
#define FND_TYPE_FILE 1u
#define FND_TYPE_DIR 2u
#define FND_TYPE_DEVICE 3u
#define FND_TYPE_CONSOLE 4u
#define FND_STAT_READ_ONLY 1u
#define FND_STAT_SEEKABLE 2u

#define FND_DIRENT_HEAD 72u
#define FND_DIRENT_MAX 1096u

#define FND_VOLINFO_BYTES 32u
#define FND_VOL_FS2 1u
#define FND_VOL_DSFS 2u
#define FND_VOL_RAM 3u
#define FND_VOL_VIRTUAL 4u
#define FND_VOL_READ_ONLY 1u
#define FND_VOL_NO_SLOT 0xFFFFFFFFu

/* §8.2, §8.3 console. */
#define FND_CON_LINE 0u
#define FND_CON_RAW 1u
#define FND_CON_SET_MODE 0x1000u
#define FND_CON_READ_KEY 0x1001u
#define FND_CON_NOWAIT 1u /* CON_READ_KEY a0: FND_END instead of waiting */
#define FND_CON_BREAK_WAIT 0x1002u
#define FND_CON_CLEAR 0x1003u
#define FND_CON_GET_SIZE 0x1004u
#define FND_CON_SET_CURSOR 0x1005u
#define FND_CON_GET_CURSOR 0x1006u
#define FND_CON_SET_COLOR 0x1007u

/* §11.6 */
#define FND_EXEC_IMAGE_MAX 0x100000u

/* §11.1: the end of a listing or of the input. */
#define FND_END 1

/* §9.3 of the kernel ABI: roles of the startup block. */
#define FND_ROLE_STDIN 1u
#define FND_ROLE_STDOUT 2u
#define FND_ROLE_STDERR 3u
#define FND_ROLE_FS 4u
#define FND_ROLE_CONSOLE 5u

/* §8.3: console key codes of CON_READ_KEY and modifier bits. */
#define FND_KEY_ENTER 1u
#define FND_KEY_BACKSPACE 2u
#define FND_KEY_DELETE 3u
#define FND_KEY_LEFT 4u
#define FND_KEY_RIGHT 5u
#define FND_KEY_UP 6u
#define FND_KEY_DOWN 7u
#define FND_KEY_HOME 8u
#define FND_KEY_END 9u
#define FND_KEY_TAB 10u
#define FND_KEY_ESC 11u
#define FND_KEY_CHAR 0x100u

#define FND_MOD_SHIFT 1u
#define FND_MOD_CTRL 2u
#define FND_MOD_ALT 4u

/* Kernel ABI §9 sys_info. */
#define FND_INFO_SYSTEM 0u
#define FND_INFO_PROCESSES 1u
#define FND_INFO_CPU 2u
#define FND_INFO_MEMORY 3u
#define FND_CPUINFO_HEADER_BYTES 16u
#define FND_CPUINFO_RECORD_BYTES 8u
#define FND_SYSINFO_BYTES 32u
#define FND_PROCINFO_BYTES 48u
#define FND_MEMINFO_BYTES 16u

#endif
