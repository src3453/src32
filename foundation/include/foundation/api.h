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
