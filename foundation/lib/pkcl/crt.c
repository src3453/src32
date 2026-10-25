/* pkcl の起動コード（doc/spec_pkcl.md §3.3）。`--entry pkcl_start` で指定する。
 * 起動ブロックから argc/argv を取り出し、`int main(int, char **)` を呼んで、
 * 戻り値で終了する。 */
#define _GNU_SOURCE 1 /* libc のヘッダに O_CLOEXEC などを出させる */
#include <foundation/fnd.h>
#include <pkcl/pkcl.h>

extern int main(int argc, char **argv);
#ifdef PKCL_LIBC
extern void exit(int code) __attribute__((noreturn));
#endif

/* 環境変数。起動ブロック（version 2）の env から作る。なければ空。 */
static char *empty_environ[1];
char **environ = empty_environ;

void pkcl_start(void) {
    fnd_init();
    pkcl_init();
    uint32_t argc = fnd_argc();
    char **argv = fnd_mem_alloc((argc + 1u) * sizeof(char *));
    if (!argv)
        fnd_exit(FND_E_NO_MEMORY);
    for (uint32_t i = 0; i < argc; ++i)
        argv[i] = (char *)fnd_argv(i); /* 起動ブロックは書き換え可能なヒープ上にある */
    argv[argc] = 0;
    uint32_t envc = fnd_envc();
    if (envc) {
        char **env = fnd_mem_alloc((envc + 1u) * sizeof(char *));
        if (!env)
            fnd_exit(FND_E_NO_MEMORY);
        for (uint32_t i = 0; i < envc; ++i)
            env[i] = (char *)fnd_env(i);
        env[envc] = 0;
        environ = env;
    }
#ifdef PKCL_LIBC
    exit(main((int)argc, argv)); /* atexit と stdio の flush を libc に任せる */
#else
    fnd_exit(main((int)argc, argv));
#endif
}
