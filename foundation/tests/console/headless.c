/* Any accidental physical MMIO dereference faults on this host. */
#include <assert.h>
#include <string.h>
#define FND_CON_UART_ONLY
#include "../../services/console/con_pcg.c"
int main(void) {
    uint32_t x, y;
    fnd_key_t key;
    fnd_keyboard state;
    assert(con_pcg_init() == FND_OK);
    assert(con_pcg_columns() == 80);
    const char *s = "abc\n\033[5;10Hxy";
    con_pcg_write((const uint8_t *)s, (uint32_t)strlen(s));
    con_pcg_get_cursor(&x, &y);
    assert(x == 11 && y == 4);
    con_pcg_set_cursor(7, 9);
    con_pcg_get_cursor(&x, &y);
    assert(x == 7 && y == 9);
    assert(con_pcg_set_display(FND_CON_DISPLAY_40) == FND_E_UNSUPPORTED);
    assert(con_pcg_set_display(FND_CON_DISPLAY_8X16) == FND_E_UNSUPPORTED);
    assert(!con_pcg_poll_key(&key));
    con_pcg_key_state(&state);
    for (uint32_t i = 0; i < 8; i++) assert(!state.down[i] && !state.pressed[i] && !state.released[i]);
    for (unsigned i = 0; i < 40; i++) con_pcg_write((const uint8_t *)"line\n", 5);
    con_pcg_clear();
    con_pcg_get_cursor(&x, &y);
    assert(x == 0 && y == 0);
    return 0;
}
