/* Real U-mode COW stores, preserved handles, nested fork and kernel output. */
#include <foundation/sys.h>
#define CHECK(x) do { if (!(x)) { fnd_sys_proc_exit(__LINE__); for (;;) {} } } while (0)
static volatile uint32_t data[3072] __attribute__((aligned(4096)));
static volatile uint32_t output[2048] __attribute__((aligned(4096)));
static uint32_t thread_stack[256] __attribute__((aligned(8)));
static void sleeper(void) { fnd_sys_sleep(10); fnd_sys_thread_exit(7); for (;;) {} }
static void wait_child(int32_t h, int32_t expected) {
    int32_t code = 0;
    CHECK(h > 0);
    CHECK(fnd_sys_wait(h, FND_FOREVER, &code) == FND_OK);
    CHECK(code == expected);
    CHECK(fnd_sys_handle_close(h) == FND_OK);
}
int main(void) {
#ifdef SLOT_TEST
    CHECK(fnd_sys_proc_fork() == FND_E_UNSUPPORTED);
#else
    /* Keep a hole before the inherited port to test exact handle numbering. */
    int32_t hole = fnd_sys_port_create();
    int32_t port = fnd_sys_port_create();
    CHECK(hole > 0 && port > hole);
    CHECK(fnd_sys_handle_close(hole) == FND_OK);
    data[0] = 11; data[1024] = 22; data[2048] = 33;
    output[1023] = 99; output[1024] = 98;
    for (unsigned round = 0; round < 3; ++round) {
        int32_t child = fnd_sys_proc_fork();
        CHECK(child >= 0);
        if (!child) {
            CHECK(data[0] == 11 && data[1024] == 22 && data[2048] == 33);
            CHECK(fnd_sys_handle_close(port) == FND_OK);
            /* First write here is by S-mode sys_info, crossing two pages. */
            CHECK(fnd_sys_info(0, &output[1023], 32) == 32);
            CHECK(output[1023] == 1 && output[1024] == 0);
            data[0] = 44;
            int32_t grandchild = fnd_sys_proc_fork();
            CHECK(grandchild >= 0);
            if (!grandchild) {
                CHECK(data[0] == 44 && data[1024] == 22);
                data[0] = 55; data[1024] = 66;
                fnd_sys_proc_exit(43);
                for (;;) {}
            }
            wait_child(grandchild, 43);
            CHECK(data[0] == 44 && data[1024] == 22);
            /* Last owner: restore W without allocating a copy. */
            data[2048] = 77;
            fnd_sys_proc_exit(42);
            for (;;) {}
        }
        data[2048] = 34; /* parent store must invalidate its pre-fork W TLB */
        wait_child(child, 42);
        CHECK(data[0] == 11 && data[1024] == 22 && data[2048] == 34);
        data[2048] = 33;
        CHECK(output[1023] == 99 && output[1024] == 98);
    }
    CHECK(fnd_sys_handle_close(port) == FND_OK);
    /* Growing within an existing shared page must not restore W on it. */
    int32_t heap = fnd_sys_mem_grow(8);
    CHECK(heap > 0);
    volatile uint32_t *h = (volatile uint32_t *)(uintptr_t)heap;
    h[0] = 123;
    int32_t grown = fnd_sys_proc_fork();
    CHECK(grown >= 0);
    if (!grown) {
        CHECK(fnd_sys_mem_grow(8) == heap + 8);
        h[0] = 456; h[2] = 789;
        fnd_sys_proc_exit(42);
        for (;;) {}
    }
    wait_child(grown, 42);
    CHECK(h[0] == 123 && fnd_sys_mem_grow(0) == heap + 8);
    /* Kill must release the child's shared pages and PROCESS references. */
    int32_t killed = fnd_sys_proc_fork();
    CHECK(killed >= 0);
    if (!killed) { for (;;) fnd_sys_sleep(10); }
    CHECK(fnd_sys_proc_kill(killed) == FND_OK);
    wait_child(killed, FND_E_KILLED);
    /* Real IPC delivery writes a COW destination in the waiting parent. */
    port = fnd_sys_port_create();
    int32_t ch = fnd_sys_chan_create(port, 77);
    CHECK(port > 0 && ch > 0);
    int32_t sender = fnd_sys_proc_fork();
    CHECK(sender >= 0);
    if (!sender) {
        fnd_msg req = {0}, rep = {0};
        req.buf = (uint32_t)(uintptr_t)&data[0]; req.len = 4;
        CHECK(fnd_sys_ipc_call(ch, &req, &rep) == FND_OK);
        CHECK(rep.a0 == 88);
        fnd_sys_proc_exit(42);
        for (;;) {}
    }
    fnd_recv rcv = {0};
    rcv.data = (uint32_t)(uintptr_t)&output[1023]; rcv.cap = 8;
    rcv.timeout = FND_FOREVER;
    CHECK(fnd_sys_ipc_recv(port, &rcv) == FND_OK);
    CHECK(rcv.badge == 77 && output[1023] == 11);
    fnd_msg reply = {0}; reply.a0 = 88;
    CHECK(fnd_sys_ipc_reply(rcv.token, &reply) == FND_OK);
    wait_child(sender, 42);
    CHECK(fnd_sys_handle_close(ch) == FND_OK);
    CHECK(fnd_sys_handle_close(port) == FND_OK);
    /* No room for the parent's new PROCESS handle: atomic LIMIT. */
    int32_t full = fnd_sys_port_create();
    CHECK(full == 1);
    unsigned count = 1;
    while (fnd_sys_handle_dup(full) > 0) ++count;
    CHECK(count > 0 && fnd_sys_proc_fork() == FND_E_LIMIT);
    for (unsigned h = 1; h <= count; ++h)
        CHECK(fnd_sys_handle_close(h) == FND_OK);
    /* True RX remains protected after fork. */
    int32_t child = fnd_sys_proc_fork();
    CHECK(child >= 0);
    if (!child) {
        *(volatile uint32_t *)(uintptr_t)main = 0;
        fnd_sys_proc_exit(1);
        for (;;) {}
    }
    wait_child(child, (int32_t)0x8006000Au);
    int32_t th = fnd_sys_thread_create(sleeper, thread_stack + 256, 0);
    CHECK(th > 0);
    CHECK(fnd_sys_proc_fork() == FND_E_BUSY);
    int32_t code;
    CHECK(fnd_sys_wait(th, FND_FOREVER, &code) == FND_OK && code == 7);
    CHECK(fnd_sys_proc_fork() == FND_E_ACCESS); /* retained THREAD handle */
    CHECK(fnd_sys_handle_close(th) == FND_OK);
#endif
    fnd_sys_proc_exit(42);
    for (;;) {}
}

