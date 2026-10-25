/* Builtin user code: only its granted code and stack/mailbox are accessible. */
#include <foundation/sys.h>

void irq_user_begin(void) {}

#define CHECK(expr, code) do { if (!(expr)) { fnd_sys_proc_exit(code); for (;;) {} } } while (0)

void irq_owner(volatile uint32_t *box) {
    int32_t port = fnd_sys_port_create();
    CHECK(port > 0, 1);
    CHECK(fnd_sys_irq_bind(16u, port) == FND_E_BAD_ARG, 2);
    CHECK(fnd_sys_irq_ack(0xFFFFFFFFu) == FND_E_BAD_ARG, 3);
    for (uint32_t irq = 0; irq < 16u; ++irq) {
        if (irq == 0u || irq == 3u || (irq >= 5u && irq <= 12u)) continue;
        CHECK(fnd_sys_irq_bind(irq, port) == FND_E_ACCESS, 4);
        CHECK(fnd_sys_irq_ack(irq) == FND_E_ACCESS, 5);
    }
    CHECK(fnd_sys_irq_bind(0, 0) == FND_E_BAD_HANDLE, 6);
    int32_t chan = fnd_sys_chan_create(port, 1);
    CHECK(chan > 0 && fnd_sys_irq_bind(0, chan) == FND_E_BAD_HANDLE, 7);
    CHECK(fnd_sys_handle_close(chan) == 0, 8);
    fnd_recv gone = {0};
    CHECK(fnd_sys_ipc_recv(port, &gone) == 0 && gone.msg.op == FND_NOTIFY_CLIENT_GONE, 8);
    for (uint32_t irq = 3; irq <= 12; ++irq) {
        if (irq == 4) continue;
        CHECK(fnd_sys_irq_bind(irq, port) == 0, 9);
    }
    CHECK(fnd_sys_irq_bind(0, port) == 0, 10);
    CHECK(fnd_sys_irq_bind(0, port) == FND_E_BUSY, 11);
    CHECK(fnd_sys_irq_ack(5) == FND_E_BAD_ARG, 12);
    box[1] = (uint32_t)port;
    box[0] = 1;
    fnd_recv r = {0};
    r.timeout = FND_FOREVER;
    CHECK(fnd_sys_ipc_recv(port, &r) == 0, 13); /* real VSync notification */
    CHECK(r.msg.op == FND_NOTIFY_IRQ && r.msg.a0 == 0 && r.token == 0 &&
          r.badge == 0 && r.reply_cap == 0 && r.total == 0 && r.msg.len == 0 &&
          r.msg.flags == 0 && r.msg.a1 == 0 && r.msg.a2 == 0 && r.msg.a3 == 0, 14);
    box[0] = 2;
    fnd_sys_sleep(5);
    r.timeout = 0;
    CHECK(fnd_sys_ipc_recv(port, &r) == FND_E_WOULD_BLOCK, 15); /* coalesced */
    CHECK(fnd_sys_irq_ack(0) == 0, 16);
    CHECK(fnd_sys_irq_ack(0) == FND_E_BAD_ARG, 17);
    box[0] = 3;
    fnd_sys_sleep(5);
    CHECK(fnd_sys_ipc_recv(port, &r) == 0 && r.msg.op == FND_NOTIFY_IRQ &&
          r.msg.a0 == 0 && r.token == 0, 18); /* queued before recv */
    CHECK(fnd_sys_irq_ack(5) == FND_E_BAD_ARG, 25); /* cannot discard unread notification */
    CHECK(fnd_sys_ipc_recv(port, (void *)0x100u) == FND_E_ACCESS, 26);
    CHECK(fnd_sys_ipc_recv(port, &r) == 0 && r.msg.op == FND_NOTIFY_IRQ && r.msg.a0 == 5, 27);
    CHECK(fnd_sys_irq_ack(5) == 0, 28);
    CHECK(fnd_sys_ipc_recv(port, &r) == 0 && r.msg.op == FND_NOTIFY_IRQ && r.msg.a0 == 6, 29);
    CHECK(fnd_sys_ipc_recv(port, &r) == FND_E_WOULD_BLOCK, 33);
    CHECK(fnd_sys_irq_ack(6) == 0, 34);
    box[0] = 5;
    r.timeout = FND_FOREVER;
    CHECK(fnd_sys_ipc_recv(port, &r) == 0 && r.msg.op == FND_NOTIFY_IRQ &&
          r.msg.a0 == 6 && r.token == 0, 35); /* injected only once blocked in recv */
    int32_t dup = fnd_sys_handle_dup(port);
    CHECK(dup > 0 && fnd_sys_handle_close(port) == 0, 19);
    CHECK(fnd_sys_irq_bind(0, dup) == FND_E_BUSY, 20);
    CHECK(fnd_sys_handle_close(dup) == 0, 21); /* final close drops pending IRQs */
    CHECK(fnd_sys_irq_ack(0) == FND_E_NOT_FOUND, 22);
    port = fnd_sys_port_create();
    CHECK(port > 0 && fnd_sys_irq_bind(0, port) == 0, 23);
    CHECK(fnd_sys_irq_bind(5, port) == 0, 24);
    box[1] = (uint32_t)port;
    box[0] = 4;
    fnd_sys_sleep(5); /* kernel retains a duplicate port in another process */
    fnd_sys_proc_exit(42); /* must release IRQ even with the port still alive */
    for (;;) {}
}

void irq_unprivileged(volatile uint32_t *box) {
    int32_t port = fnd_sys_port_create();
    CHECK(port > 0, 30);
    CHECK(fnd_sys_irq_bind(0, port) == FND_E_ACCESS, 31);
    CHECK(fnd_sys_irq_ack(0) == FND_E_ACCESS, 32);
    fnd_sys_proc_exit(42);
    for (;;) {}
}

void irq_keeper(volatile uint32_t *box) {
    for (;;) fnd_sys_sleep(1);
}

void irq_user_end(void) {}
