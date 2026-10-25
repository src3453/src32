#ifndef PKCL_SIGNAL_H
#define PKCL_SIGNAL_H
#include_next <signal.h>
#ifndef SA_RESTART
#define SA_RESTART 0x10000000
#endif
#ifndef SA_NODEFER
#define SA_NODEFER 0x40000000
#endif
#ifndef SA_RESETHAND
#define SA_RESETHAND 0x80000000
#endif
int sigisemptyset(const sigset_t *set);
#endif
