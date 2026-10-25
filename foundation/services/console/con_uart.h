#ifndef FOUNDATION_CON_UART_H
#define FOUNDATION_CON_UART_H
#include <foundation/fnd.h>

void con_uart_write(const uint8_t *text, uint32_t len);
/* now is the low scheduler tick (100 Hz); partial escape sequences wait 5 ticks. */
int con_uart_poll_key(fnd_key_t *key, uint32_t now);
void con_uart_clear(void);
void con_uart_cursor(uint32_t x, uint32_t y);
void con_uart_color(uint32_t front, uint32_t back);
#endif
