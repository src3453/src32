/* setjmp / longjmp はコンパイラ側の対応がまだない（doc/spec_pkcl.md §7 G7）。
 * setjmp は常に 0 を返し、longjmp は戻り先がないので終了する。 */
#include <setjmp.h>
#include <foundation/fnd.h>

int setjmp(jmp_buf env) {
    (void)env;
    return 0;
}
void longjmp(jmp_buf env, int value) {
    (void)env, (void)value;
    fnd_con_write("pkcl: longjmp is not supported\n");
    fnd_exit(127);
}
