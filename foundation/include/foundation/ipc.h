#ifndef FOUNDATION_IPC_H
#define FOUNDATION_IPC_H
#include <stdint.h>

/* IPC structures and spawn records of the kernel ABI (doc/spec_Foundation.md
 * §8, §9.1, §9.2, §9.3). Every field is a big-endian u32, which is the
 * native order of SRC32, so these structs are the wire layout. */

typedef struct fnd_msg {
    uint32_t op;
    uint32_t flags; /* FND_MSG_XFER_HANDLE */
    uint32_t a0, a1, a2, a3;
    uint32_t buf;
    uint32_t len;
} fnd_msg;

#define FND_MSG_XFER_HANDLE 1u
#define FND_IPC_MAX_DATA 4096u

typedef struct fnd_recv {
    fnd_msg msg;
    uint32_t data;      /* in: receive buffer */
    uint32_t cap;       /* in: its capacity */
    uint32_t token;     /* out: 0 for a notification */
    uint32_t badge;     /* out */
    uint32_t reply_cap; /* out: the client's reply data capacity */
    uint32_t total;     /* out: the sender's len */
    uint32_t timeout;   /* in: §7.1 */
    uint32_t reserved;
} fnd_recv;

/* §8.5 kernel notifications. */
#define FND_NOTIFY_IRQ 0xFFFF0001u
#define FND_NOTIFY_CLIENT_GONE 0xFFFF0002u

/* §9.1 capabilities. */
#define FND_CAP_PRIO 1u
#define FND_CAP_IRQ 2u
#define FND_CAP_DISK 4u
#define FND_CAP_SVC 8u
#define FND_CAP_MMIO 16u
#define FND_CAP_ALL 31u

/* §9.2 proc_spawn (48 bytes). */
typedef struct fnd_spawn {
    uint32_t image, image_len;
    uint32_t startup, startup_len;
    uint32_t handles, handle_count;
    uint32_t priority;
    uint32_t caps;
    uint32_t grants, grant_count; /* 12-byte {start, end, perm} entries */
    uint32_t name;                /* NUL-terminated, at most 15 bytes */
    uint32_t libs;                /* fnd_spawn_libs, or 0 (§9.5) */
} fnd_spawn;

/* The WALs a v3 WAX and its WALs import (§9.2, §9.5). */
typedef struct fnd_spawn_lib {
    uint32_t image, image_len;
} fnd_spawn_lib;
typedef struct fnd_spawn_libs {
    uint32_t count; /* at most FND_SPAWN_MAX_LIBS */
    uint32_t reserved;
    /* followed by count fnd_spawn_lib */
} fnd_spawn_libs;

#define FND_SPAWN_MAX_LIBS 16u
#define FND_SPAWN_MAX_HANDLES 16u
#define FND_STARTUP_MAX 4096u
/* Largest image proc_spawn copies to its staging area. */
#define FND_SPAWN_IMAGE_MAX 0x180000u

/* §9.3 startup block header. */
#define FND_STARTUP_MAGIC 0x464E4453u /* FNDS */
#define FND_STARTUP_VERSION 2u /* 1: without env_count and env_off (a reader treats them as 0) */
typedef struct fnd_startup {
    uint32_t magic, version, size;
    uint32_t argc, argv_off;
    uint32_t cwd_off;
    uint32_t role_count, roles_off; /* (role, handle) pairs */
    uint32_t env_count, env_off;    /* version 2: "NAME=value" strings, like argv */
} fnd_startup;

/* §6: handle values are 1..63. */
#define FND_MAX_HANDLES 63u

#endif
