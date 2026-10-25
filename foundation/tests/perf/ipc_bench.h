#ifndef IPC_BENCH_H
#define IPC_BENCH_H
#include <stdint.h>
#ifndef BENCH_ROUNDS
#define BENCH_ROUNDS 64u
#endif
#ifndef BENCH_SERVER_PRIORITY
#define BENCH_SERVER_PRIORITY FND_PRIO_SERVICE
#endif
#define BENCH_CASES 11u
#define BENCH_RAM 0x10000u
struct bench_box {
    uint32_t handle, done;
    uint32_t samples[BENCH_CASES][3]; /* elapsed, kernel, idle, modulo 2^32 */
};
void bench_user_begin(void), bench_user_end(void);
void bench_client(struct bench_box *), bench_server(struct bench_box *);
#endif
