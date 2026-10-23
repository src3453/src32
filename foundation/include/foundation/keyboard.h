#ifndef FOUNDATION_KEYBOARD_H
#define FOUNDATION_KEYBOARD_H
#include <foundation/fnd.h>
#define FND_CON_KEY_STATE 0x1008u
/* HID page 7 usage bits; edges since the previous query.
 * One shared physical keyboard. This does not consume CON_READ_KEY. */
typedef struct { uint32_t down[8], pressed[8], released[8]; } fnd_keyboard;
static inline fnd_status fnd_con_key_state(fnd_keyboard *state) {
    fnd_msg rep;
    fnd_status st = fnd_call(fnd_console(), FND_CON_KEY_STATE, 0, 0, 0,
                             state, sizeof *state, &rep);
    return st == FND_OK && rep.len != sizeof *state ? FND_E_IO : st;
}
#endif
