#include <foundation/fnd.h>
#include <foundation/sys.h>
#define CHECK(x) do { if (!(x)) fnd_exit(__LINE__); } while (0)
static void out(const char *s) { CHECK(fnd_con_write(s) == FND_OK); }
static void cursor(uint32_t x, uint32_t y) {
    fnd_msg reply;
    CHECK(fnd_call(fnd_console(), FND_CON_GET_CURSOR, 0, 0, 0, 0, 0, &reply) == FND_OK);
    CHECK(reply.a0 == x && reply.a1 == y);
}
int main(void) {
    fnd_init();
    CHECK(fnd_con_clear() == FND_OK);
    /* Two separately opened streams share the screen's parsing state. */
    fnd_handle other;
    CHECK(fnd_file_open("/dev/con", FND_OPEN_WRITE, FND_MODE_DEFAULT, &other) == FND_OK);
    out("\033[3;");
    CHECK(fnd_write(other, "7H", 2, 0) == FND_OK);
    CHECK(fnd_close(other) == FND_OK);
    cursor(6, 2);
    out("abc\033"); out("["); out("4;"); out("6H"); cursor(5, 3);
    out("\033[2A\033[3D"); cursor(2, 1);
    out("\033[s\033[9999;9999H"); cursor(79, 29);
    out("\033[u"); cursor(2, 1);
    out("\033[2J"); cursor(2, 1);
    out("\033[999999999999H\033[?123h\033[0 q"); cursor(2, 1);
    for (uint32_t i = 0; i < 34; ++i) out("scroll\n");
    out("\033[H\033[2Jafter scroll"); cursor(12, 0);
    /* Color uses a0/a1, so use the lower-level call structure for both. */
    fnd_msg msg = {FND_CON_SET_COLOR, 0, 1, 4, 0, 0, 0, 0};
    fnd_msg rep = {0};
    CHECK(fnd_sys_ipc_call(fnd_console(), &msg, &rep) == FND_OK && (int32_t)rep.op == FND_OK);
    msg.a0 = 16;
    CHECK(fnd_sys_ipc_call(fnd_console(), &msg, &rep) == FND_OK && (int32_t)rep.op == FND_E_BAD_ARG);
    CHECK(fnd_con_clear() == FND_OK);
    out("C\033[1;7mR\033[0mD\033[?25l\033[?25h\nconsole guest passed\n");
    return 0;
}
