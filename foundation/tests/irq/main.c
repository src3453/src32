/* Real host-frame VSync -> kernel trap -> IPC -> user ACK, in slots and VM.
 * Kernel checks inspect IRQC while the user leaves a notification unacked. */
#include "internal.h"

void irq_user_begin(void), irq_user_end(void);
void irq_owner(volatile uint32_t *), irq_unprivileged(volatile uint32_t *), irq_keeper(volatile uint32_t *);
static struct fnd_proc *owner, *unpriv, *keeper;
static uint32_t failures, observed, retained;
static uint32_t mailbox;
#define check(ok) do { if (!(ok) && !failures) failures = __LINE__; } while (0)

static uint32_t on_tick(uint32_t ticks) {
    if (owner->state == FND_PROC_LIVE) {
        uint32_t stage = mm_read32(owner, mailbox);
        if (stage == 2 || stage == 3 || stage == 4) {
            if (FND_REG16(FND_IRQC_PENDING) & 1u) {
                check(!(FND_REG16(FND_IRQC_ENABLE) & 1u));
                observed |= 1u << stage;
            }
        }
        if (stage == 2 && !(observed & 1u)) {
            int32_t port = sys_port_create(keeper);
            check(port > 0);
            check(sys_irq_bind(keeper, 0, port) == FND_E_BUSY);
            check(sys_irq_ack(keeper, 0) == FND_E_ACCESS);
            check(sys_handle_close(keeper, port) == 0);
            observed |= 1u;
        }
        if (stage == 3 && !(observed & 2u)) {
            /* Expansion sources are synthetic here; IRQ0 above uses the real
             * host-frame interrupt. Repeated notifications must coalesce. */
            check(ipc_irq_notify(5) == 1 && ipc_irq_notify(5) == 1);
            check(ipc_irq_notify(6) == 1);
            observed |= 2u;
        }
        if (stage == 5 && !(observed & 32u)) {
            for (unsigned i = 0; i < FND_MAX_THREADS; ++i) {
                struct fnd_thread *t = &fnd_threads[i];
                if (t->proc == owner && t->state == T_BLOCKED && t->reason == B_RECV) {
                    check(ipc_irq_notify(6) == 1);
                    check(t->state == T_READY); /* synchronous notification wakes recv */
                    check(ipc_irq_notify(7) == 1); /* unread notification dropped on close */
                    observed |= 32u;
                    break;
                }
            }
        }
        if (stage == 4 && !retained) {
            retained = (uint32_t)handle_dup_into(owner, mm_read32(owner, mailbox + 4u), keeper);
            check((int32_t)retained > 0);
        }
    } else {
        if (owner->exit_code != 42) return 0x300u + (uint32_t)owner->exit_code;
        if (unpriv->state == FND_PROC_EXITED && unpriv->exit_code != 42)
            return 0x400u + (uint32_t)unpriv->exit_code;
        check(owner->exit_code == 42 && owner->fault_epc == 0);
        check(unpriv->state == FND_PROC_EXITED && unpriv->exit_code == 42);
        check((observed & 63u) == 63u && retained > 0);
        check((FND_REG16(FND_IRQC_ENABLE) & 0x1FE9u) == 0);
        /* VSync may latch again while disabled after exit; ENABLE and the
         * binding, rather than the next frame's PENDING, prove release. */
        check(sys_irq_ack(keeper, 0) == FND_E_NOT_FOUND);
        check(sys_irq_bind(keeper, 0, retained) == 0);
        check(sys_irq_bind(keeper, 5, retained) == 0);
        /* A killed owner also releases bindings, even with queued events. */
        check(ipc_irq_notify(5) == 1);
        proc_exit(keeper, FND_E_KILLED, 0);
        check((FND_REG16(FND_IRQC_ENABLE) & 0x21u) == 0);
        check(ipc_irq_notify(5) == 0);
        return failures ? 0x100u + failures : 42u;
    }
    return ticks > 80 ? 0x200u : 0;
}

static struct fnd_proc *spawn(uint32_t slot, void (*fn)(volatile uint32_t *)) {
    uint32_t box = fnd_slot_end(slot) - 0x4000u;
    int32_t pid = fnd_proc_create_builtin("irq", slot, FND_PRIO_SERVICE, 0x4000u,
        (uint32_t)(uintptr_t)irq_user_begin, (uint32_t)(uintptr_t)irq_user_end,
        (uint32_t)(uintptr_t)fn, box);
    return pid > 0 ? fnd_proc_get((uint32_t)pid) : 0;
}

int main(void) {
    owner = spawn(5, irq_owner);
    unpriv = spawn(4, irq_unprivileged);
    keeper = spawn(6, irq_keeper);
    if (!owner || !unpriv || !keeper) return 2;
#ifdef FND_MM_NOMMU_ONLY
    if (mm_vm) return 3;
#else
    if (!mm_vm) return 4; /* this test's default run explicitly checks VM */
#endif
    owner->caps = keeper->caps = FND_CAP_IRQ;
    mailbox = fnd_slot_end(5) - 0x4000u;
    fnd_set_tick_hook(on_tick);
    fnd_kernel_run();
}
