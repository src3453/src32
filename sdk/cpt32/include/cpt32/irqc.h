#ifndef CPT32_IRQC_H
#define CPT32_IRQC_H
#include <stdint.h>
#define IRQC_PENDING 0xFFFF0040u
#define IRQC_ENABLE 0xFFFF0044u
#define IRQC_CAUSE 0xFFFF0288u
#define IRQC_LINES 16u

typedef void (*irqc_handler)(void);
/* Call before enabling sources. Disables/acknowledges all sources, clears
 * callbacks. */
/* 全IRQの無効化・ackを行い、コールバックとVSyncカウンタを初期化します。 */
void irqc_init(void);
/* Returns 0 on success, 1 for invalid line. NULL unregisters a callback.
 * Call with source disabled. The dispatcher invokes the callback before
 * acknowledging its pending bit, so clear any device condition in the
 * callback. Unregistered IRQs are still acknowledged. */
/* IRQのコールバックを登録・解除します（0=成功、1=不正なIRQ番号）。 */
int irqc_register(unsigned line, irqc_handler handler);
/* maskで指定したIRQを有効にします。 */
void irqc_enable(uint16_t mask);
/* maskで指定したIRQを無効にします。 */
void irqc_disable(uint16_t mask);
/* maskで指定したIRQの保留ビットをクリアします。 */
void irqc_ack(uint16_t mask);
/* 有効な保留IRQのうち最優先の番号を返します（該当なしは-1）。 */
int irqc_irq_number(void);
/* Monotonic IRQ0 count, advanced by dispatcher before callback. */
/* dispatcherが処理したIRQ0の累積回数を返します。 */
uint32_t irqc_vsync_count(void);
/* Waits for an IRQ0 after this call. Requires IRQ0 enabled. No synthetic cycle
 * clock. */
/* 呼出し後の次のIRQ0を、VSyncカウンタの変化で待ちます。 */
void irqc_wait_vsync(void);
/* Runtime entry called with interrupts masked; do not call directly or enable
 * nesting. It drains all currently enabled pending IRQs in priority order.
 * The compiler installs vectors when this library is linked, before main(). */
/* 割り込み入口から呼ばれ、有効な保留IRQのコールバック実行とackを行います。 */
void irqc_dispatch(void);
#endif
