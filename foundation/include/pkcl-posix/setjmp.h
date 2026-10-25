#ifndef PKCL_SETJMP_H
#define PKCL_SETJMP_H
#include_next <setjmp.h>
/* シグナルマスクは保存しない（pkcl はシグナルを配送しない）ので、sig 付きは付かないものと同じ。 */
#ifndef sigsetjmp
typedef jmp_buf sigjmp_buf;
#define sigsetjmp(env, savemask) setjmp(env)
#define siglongjmp(env, val) longjmp(env, val)
#endif
#endif
