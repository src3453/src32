/* blk_io (doc/spec_Foundation.md §10.4): sector I/O on the IDC for
 * processes with the DISK capability. Requests run one at a time in FIFO
 * order on DMAC channel 0; the caller blocks in BLK_IO and the IDC
 * completion or error interrupt (IRQ 2, kernel-only per §10.2) wakes it. */
#include "internal.h"

#include <cpt32/idc.h>

#define IDC_IRQ_ENABLE 0x8004100Cu
#define IDC_IRQ_STATUS 0x80041010u
#define IDC_DRIVE_TYPE 0x80041028u
#define IDC_SECTOR_SIZE 0x8004102Cu
#define IDC_DRIVE_FLAGS 0x80041034u
#define IDC_CMD_ABORT 4u
#define IDC_IRQ_BITS 3u /* completion, error */

#define BLK_READ 1u
#define BLK_WRITE 2u
#define BLK_IDENTIFY 3u
#define BLK_MAX_COUNT 128u

struct blk_req {
    struct fnd_thread *thread;
    uint32_t drive, op, lba, count, buf;
};

/* The DMAC knows physical addresses only, and the pages of a user buffer
 * need not be adjacent: with V every transfer goes through the kernel's bounce
 * buffer (mm_bounce) and is copied to or from the process by software. */
static uint32_t dma_address(const struct blk_req *r) { return mm_vm ? mm_bounce() : r->buf; }

/* queue[0] is the request on the IDC while `running`. */
static struct blk_req queue[FND_MAX_THREADS];
static uint32_t queued, running;

/* §10.4: IDC error 2 -> NO_MEDIA, 3 -> BAD_ARG, 4 -> READ_ONLY, 5 and 6 -> IO.
 * 1 (bad command or drive) cannot come from a checked request; BAD_ARG. */
static int32_t idc_result(void) {
    if (!(FND_REG32(IDC_STATUS) & IDC_BIT_ERROR) && !(FND_REG32(IDC_DMA_CH0_STATUS) & IDC_DMA_BIT_ERROR))
        return FND_OK;
    /* An if chain: a switch becomes a lookup table with a poison hole for
     * code 3, which the llvm-src32 data emitter cannot encode. */
    uint32_t code = FND_REG32(IDC_ERROR_CODE);
    if (code == 2)
        return FND_E_NO_MEDIA;
    if (code == 4)
        return FND_E_READ_ONLY;
    if (code == 5 || code == 6)
        return FND_E_IO;
    return (FND_REG32(IDC_DMA_CH0_STATUS) & IDC_DMA_BIT_ERROR) ? FND_E_IO : FND_E_BAD_ARG;
}

static void idc_clear(void) {
    FND_REG32(IDC_STATUS) = IDC_CLEAR_BITS;
    FND_REG32(IDC_IRQ_STATUS) = IDC_IRQ_BITS;
    FND_REG32(IDC_DMA_CH0_STATUS) = IDC_CLEAR_BITS;
}

/* IDENTIFY completes at once (spec_IDC.md §4); it writes type,
 * sector_size, capacity and flags. */
static int32_t identify(const struct blk_req *r) {
    idc_clear();
    FND_REG32(IDC_DRIVE_SELECT) = r->drive;
    FND_REG32(IDC_COMMAND) = IDC_CMD_IDENTIFY;
    while (FND_REG32(IDC_STATUS) & IDC_BIT_BUSY) {
    }
    int32_t result = idc_result();
    idc_clear();
    if (result == FND_OK) {
        uint32_t out[4] = {FND_REG32(IDC_DRIVE_TYPE), FND_REG32(IDC_SECTOR_SIZE), FND_REG32(IDC_CAPACITY),
                           FND_REG32(IDC_DRIVE_FLAGS)};
        mm_copy_to_user(r->thread->proc, r->buf, out, sizeof out);
    }
    return result;
}

static void start(const struct blk_req *r) {
    idc_clear();
    FND_REG32(IDC_DMA_GLOBAL_CONTROL) = 1u;
    FND_REG32(IDC_DMA_CHANNEL_ENABLE) = FND_REG32(IDC_DMA_CHANNEL_ENABLE) | 1u;
    FND_REG32(IDC_IRQ_ENABLE) = IDC_IRQ_BITS;
    FND_REG16(FND_IRQC_ENABLE) = FND_REG16(FND_IRQC_ENABLE) | (1u << FND_IRQ_IDC);
    FND_REG32(IDC_DRIVE_SELECT) = r->drive;
    FND_REG32(IDC_LBA) = r->lba;
    FND_REG32(IDC_SECTOR_COUNT) = r->count;
    if (mm_vm && r->op == BLK_WRITE)
        mm_copy_from_user((void *)(uintptr_t)mm_bounce(), r->thread->proc, r->buf, r->count << 9);
    if (r->op == BLK_READ) {
        FND_REG32(IDC_DMA_CH0_SRC) = IDC_DATA;
        FND_REG32(IDC_DMA_CH0_DST) = dma_address(r);
    } else {
        FND_REG32(IDC_DMA_CH0_SRC) = dma_address(r);
        FND_REG32(IDC_DMA_CH0_DST) = IDC_DATA;
    }
    FND_REG32(IDC_DMA_CH0_COUNT) = r->count << 9;
    FND_REG32(IDC_COMMAND) = r->op == BLK_READ ? IDC_CMD_READ : IDC_CMD_WRITE;
    FND_REG32(IDC_DMA_CH0_CONTROL) = r->op == BLK_READ ? IDC_DMA_READ_CONTROL : IDC_DMA_WRITE_CONTROL;
}

static void pop(void) {
    for (uint32_t i = 1; i < queued; ++i)
        queue[i - 1] = queue[i];
    --queued;
}

/* Starts queue[0], finishing IDENTIFY requests on the way. */
static void run_queue(void) {
    while (queued && !running) {
        if (queue[0].op == BLK_IDENTIFY) {
            int32_t result = identify(&queue[0]);
            sched_wake(queue[0].thread, result);
            pop();
            continue;
        }
        running = 1;
        start(&queue[0]);
    }
}

int blk_pending(void) { return running && (FND_REG32(IDC_IRQ_STATUS) & IDC_IRQ_BITS) != 0; }

void blk_irq(void) {
    if (!running) {
        idc_clear();
        return;
    }
    /* The IDC completes after the last sector; let the DMAC finish too. */
    while ((FND_REG32(IDC_STATUS) & IDC_BIT_BUSY) || (FND_REG32(IDC_DMA_CH0_STATUS) & IDC_DMA_BIT_BUSY)) {
    }
    int32_t result = idc_result();
    idc_clear();
    running = 0;
    if (mm_vm && result == FND_OK && queue[0].op == BLK_READ)
        mm_copy_to_user(queue[0].thread->proc, queue[0].buf, (const void *)(uintptr_t)mm_bounce(),
                        queue[0].count << 9);
    sched_wake(queue[0].thread, result);
    pop();
    run_queue();
}

/* §9.4 step 2: abort a transfer into the exiting process's memory and wait
 * for it before the memory is released; drop its queued requests. */
void blk_proc_exited(struct fnd_proc *proc) {
    uint32_t keep = 0;
    if (running && queue[0].thread->proc == proc) {
        FND_REG32(IDC_COMMAND) = IDC_CMD_ABORT;
        while ((FND_REG32(IDC_STATUS) & IDC_BIT_BUSY) || (FND_REG32(IDC_DMA_CH0_STATUS) & IDC_DMA_BIT_BUSY)) {
        }
        idc_clear();
        running = 0;
        pop();
    }
    for (uint32_t i = 0; i < queued; ++i)
        if (queue[i].thread->proc != proc)
            queue[keep++] = queue[i];
    queued = keep;
    run_queue();
}

int32_t sys_blk_io(struct fnd_thread *thread, uint32_t blk) {
    struct fnd_proc *proc = thread->proc;
    if (!(proc->caps & FND_CAP_DISK))
        return FND_E_ACCESS;
    if (blk & 3u)
        return FND_E_BAD_ARG;
    if (!proc_user_range_ok(proc, blk, 24, CPT32_PERM_R))
        return FND_E_ACCESS;
    uint32_t w[6];
    mm_copy_from_user(w, proc, blk, sizeof w);
    struct blk_req r = {thread, w[0], w[1], w[2], w[3], w[4]};
    if (w[5] || r.drive > 3 || r.op < BLK_READ || r.op > BLK_IDENTIFY)
        return FND_E_BAD_ARG;
    uint32_t bytes = 16;
    if (r.op != BLK_IDENTIFY) {
        if (r.count == 0 || r.count > BLK_MAX_COUNT)
            return FND_E_BAD_ARG;
        bytes = r.count << 9;
    }
    if (r.op != BLK_WRITE && (r.buf & 3u))
        return FND_E_BAD_ARG;
    if (!proc_user_range_ok(proc, r.buf, bytes, r.op == BLK_WRITE ? CPT32_PERM_R : CPT32_PERM_W))
        return FND_E_ACCESS;
    if (queued >= FND_MAX_THREADS)
        return FND_E_LIMIT;
    if (!queued && r.op == BLK_IDENTIFY)
        return identify(&r);
    queue[queued++] = r;
    sched_block(thread, B_BLK_IO, FND_FOREVER);
    run_queue();
    return FND_OK; /* the result comes with sched_wake */
}
