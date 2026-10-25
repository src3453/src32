/* pkcl の起動コード（doc/spec_pkcl.md §3.3）。`--entry pkcl_start` で指定する。
 * 起動ブロックから argc/argv を取り出し、`int main(int, char **)` を呼んで、
 * 戻り値で終了する。 */
#include <foundation/fnd.h>
#include <pkcl/pkcl.h>

extern int main(int argc, char **argv);

/* 環境変数。起動ブロックにはまだ環境がない（doc/spec_pkcl.md §7 G2）ので空から始める。 */
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
    fnd_exit(main((int)argc, argv));
}
