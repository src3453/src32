/* Keep all user code between these markers, like tests/irq/user.c.
 * No writable globals or calls outside the granted code region. */
#include <foundation/sys.h>
#include "ipc_bench.h"

void bench_user_begin(void) {}
#define CHECK(expr) do { if (!(expr)) { fnd_sys_proc_exit(__LINE__); for (;;) {} } } while (0)

void bench_server(struct bench_box *box) {
    uint8_t storage_space[8192 + 4];
    uint8_t *storage = (uint8_t *)(((uintptr_t)storage_space + 4095u) & ~4095u);
    fnd_recv r = {0};
    /* Match client alignment, including the page-crossing unaligned case. */
    r.data = (uint32_t)(uintptr_t)storage;
    r.cap = 4096;
    r.timeout = FND_FOREVER;
    CHECK(fnd_sys_ipc_recv(box->handle, &r) == FND_OK);
    for (;;) {
        CHECK(r.token && r.badge == 123 && r.msg.len == r.total &&
              r.msg.len <= 4096 && r.reply_cap == r.msg.len);
        fnd_msg reply = r.msg;
        reply.buf = r.data;
        uint32_t combined = r.msg.op;
        /* a1 sets the receive buffer alignment for the following request. */
        r.data = (uint32_t)(uintptr_t)storage + r.msg.a1;
        if (combined) {
            CHECK(fnd_sys_ipc_reply_recv(r.token, &reply, box->handle, &r) == FND_OK);
        } else {
            CHECK(fnd_sys_ipc_reply(r.token, &reply) == FND_OK);
            CHECK(fnd_sys_ipc_recv(box->handle, &r) == FND_OK);
        }
    }
}

void bench_client(struct bench_box *box) {
    uint8_t send_space[8192 + 4], receive_space[8192 + 4];
    uint8_t *send = (uint8_t *)(((uintptr_t)send_space + 4095u) & ~4095u);
    uint8_t *receive = (uint8_t *)(((uintptr_t)receive_space + 4095u) & ~4095u);
    for (uint32_t i = 0; i < 4096 + 4; ++i) send[i] = (uint8_t)(i * 37u + 11u);
    for (uint32_t c = 0; c < BENCH_CASES; ++c) {
        uint32_t variant = c ? (c - 1u) % 5u : 0;
        uint32_t len = variant == 0 ? 0 : variant == 1 ? 32 : variant == 2 ? 256 : 4096;
        uint32_t offset = variant == 4 ? 1 : 0;
        fnd_msg req = {0}, rep = {0};
        req.op = c > 5;
        req.a1 = offset;
        req.buf = (uint32_t)(uintptr_t)(send + offset);
        req.len = len;
        rep.buf = (uint32_t)(uintptr_t)(receive + offset);
        rep.len = len;
        uint32_t before[4], after[4];
        /* Warmup also primes the server's receive alignment before timing. */
        for (uint32_t phase = 0; phase < 2; ++phase) {
            if (phase) CHECK(fnd_sys_info(2, before, sizeof before) >= 16);
            uint32_t rounds = phase ? BENCH_ROUNDS : 8u;
            for (uint32_t i = 0; i < rounds; ++i) {
                if (!c) {
                    CHECK(fnd_sys_time_get(FND_TIME_TICK_HZ) == 100);
                } else {
                    req.a0 = i;
                    CHECK(fnd_sys_ipc_call(box->handle, &req, &rep) == FND_OK);
                    CHECK(rep.a0 == i && rep.len == len && rep.op == req.op);
                }
            }
            if (phase) CHECK(fnd_sys_info(2, after, sizeof after) >= 16);
        }
        CHECK(before[3] == 48000000 && after[3] == before[3]);
        for (uint32_t i = 0; i < 3; ++i) box->samples[c][i] = after[i] - before[i];
        /* Payload validation is outside the measured interval. */
        if (c) for (uint32_t i = 0; i < len; ++i)
            CHECK(receive[i + offset] == send[i + offset]);
    }
    box->done = 1;
    for (;;) fnd_sys_sleep(FND_FOREVER);
}
void bench_user_end(void) {}
