#ifndef FOUNDATION_FND_H
#define FOUNDATION_FND_H
#include <stdint.h>

#include <foundation/api.h>
#include <foundation/ipc.h>

/* libfnd (doc/spec_Foundation_API.md §11), the part that exists so far.
 * Every operation is an IPC call to the service that owns the object: the
 * console service for console streams, the fs service for sessions and
 * files. 64-bit sizes and positions are not used yet. */

typedef int32_t fnd_status;
typedef uint32_t fnd_handle;

typedef struct {
    uint32_t code; /* FND_KEY_* */
    uint32_t ch;   /* Unicode scalar for FND_KEY_CHAR, else 0 */
    uint32_t mods; /* FND_MOD_* */
} fnd_key_t;

/* §10.1, 64 bytes, the wire layout. */
typedef struct {
    uint32_t type, mode, uid, flags;
    uint32_t size_hi, size_lo;
    uint32_t created_sec_hi, created_sec_lo, created_ms;
    uint32_t modified_sec_hi, modified_sec_lo, modified_ms;
    uint32_t changed_sec_hi, changed_sec_lo, changed_ms;
    uint32_t reserved;
} fnd_stat_t;

/* §11.4: wire の rec_len を除き、名前を NUL 終端した1件。1100 bytes。 */
typedef struct {
    fnd_stat_t stat;
    uint32_t name_bytes, name_chars;
    char name[1025];
} fnd_dirent_t;

/* §11.6 */
typedef struct {
    uint32_t argc;
    const char *const *argv; /* argv[0] included */
    const char *cwd;         /* NULL: the current cwd */
    fnd_handle std_in, std_out, std_err; /* 0: pass our own */
    uint32_t flags;          /* reserved, 0 */
} fnd_exec_t;

/* §11.2 */
fnd_status fnd_init(void);
uint32_t fnd_argc(void);
const char *fnd_argv(uint32_t i);
fnd_handle fnd_std_in(void);
fnd_handle fnd_std_out(void);
fnd_handle fnd_std_err(void);
fnd_handle fnd_console(void);
fnd_handle fnd_fs(void);
void fnd_exit(int32_t code) __attribute__((noreturn));

/* §11.3 */
fnd_status fnd_file_open(const char *path, uint32_t flags, uint32_t mode, fnd_handle *out);
fnd_status fnd_read(fnd_handle h, void *buf, uint32_t len, uint32_t *got);
fnd_status fnd_write(fnd_handle h, const void *buf, uint32_t len, uint32_t *put);
fnd_status fnd_stream_stat(fnd_handle h, fnd_stat_t *st);
fnd_status fnd_close(fnd_handle h);
/* 保留中の変更をストリームのサービスへ同期する。 */
fnd_status fnd_sync(fnd_handle h);

/* §11.4 */
fnd_status fnd_path_stat(const char *path, fnd_stat_t *st);
/* パスで指定したファイルを削除する。 */
fnd_status fnd_file_delete(const char *path);
/* 同じ mount の中で名前を変える（移動を含む）。new_path がすでにあれば FND_E_EXISTS。 */
fnd_status fnd_file_rename(const char *old_path, const char *new_path);
/* ディレクトリを作る。mode は FND_MODE_DEFAULT を使える。 */
fnd_status fnd_dir_make(const char *path, uint32_t mode);
/* 空のディレクトリを削除する。 */
fnd_status fnd_dir_remove(const char *path);
/* 列挙用 CHANNEL を開く。fnd_close で解放する。 */
fnd_status fnd_dir_open(const char *path, fnd_handle *out);
/* 内部にためた応答から1件返す。終了は FND_END、出力引数は変更しない。 */
fnd_status fnd_dir_next(fnd_handle d, fnd_dirent_t *ent);
/* サービスの列挙位置と内部キャッシュを先頭へ戻す。 */
fnd_status fnd_dir_rewind(fnd_handle d);
fnd_status fnd_cwd_set(const char *path);
fnd_status fnd_cwd_get(char *buf, uint32_t cap, uint32_t *len);

/* §11.5 */
fnd_status fnd_con_write(const char *text);
fnd_status fnd_con_getkey(fnd_key_t *key);
fnd_status fnd_con_set_mode(uint32_t mode);
/* Nonblocking: FND_END if no key is available. */
fnd_status fnd_con_pollkey(fnd_key_t *key);
fnd_status fnd_con_clear(void);
fnd_status fnd_con_size(uint32_t *cols, uint32_t *rows);

/* §11.6 */
fnd_status fnd_proc_exec(const char *path, const fnd_exec_t *ex, fnd_handle *proc);
/* Parent: child PROCESS handle; child: 0; error: negative FND_E_*. */
int32_t fnd_proc_fork(void);
fnd_status fnd_proc_wait(fnd_handle proc, int32_t *code);
fnd_status fnd_proc_kill(fnd_handle proc);

/* §11.7 (lib/fnd/name.c) */
fnd_status fnd_name_fold(const char *name, char *out, uint32_t cap, uint32_t *len);
int fnd_name_equal(const char *a, const char *b);
int fnd_name_match(const char *pattern, const char *name);

/* §11.8 */
void *fnd_mem_alloc(uint32_t size);
void fnd_mem_free(void *p);

/* §11.9 */
const char *fnd_strerror(fnd_status code);

/* One IPC call (§2.1): sends op with a0..a3 and the data [data, data +
 * len), receives up to rep_cap bytes into rep_data. Returns the delivery
 * error or the service's result (reply op); *reply gets the whole reply. */
fnd_status fnd_call(fnd_handle h, uint32_t op, const uint32_t args[4], const void *data, uint32_t len,
                    void *rep_data, uint32_t rep_cap, fnd_msg *reply);

#endif
