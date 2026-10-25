#include <foundation/fnd.h>
#include <foundation/sys.h>
#define CHECK(x) do { if (!(x)) fnd_exit(__LINE__); } while (0)
static void out(const char *s) { CHECK(fnd_con_write(s) == FND_OK); }
static void cursor(uint32_t x, uint32_t y) {
    fnd_msg reply;
    CHECK(fnd_call(fnd_console(), FND_CON_GET_CURSOR, 0, 0, 0, 0, 0, &reply) == FND_OK);
    CHECK(reply.a0 == x && reply.a1 == y);
}
static void display(uint32_t mode, uint32_t width) {
    uint32_t args[4] = {mode, 0, 0, 0};
    fnd_msg reply;
    CHECK(fnd_call(fnd_console(), FND_CON_SET_DISPLAY, args, 0, 0, 0, 0, &reply) == FND_OK);
    CHECK(fnd_call(fnd_console(), FND_CON_GET_DISPLAY, 0, 0, 0, 0, 0, &reply) == FND_OK);
    CHECK(reply.a0 == mode);
    CHECK(fnd_call(fnd_console(), FND_CON_GET_SIZE, 0, 0, 0, 0, 0, &reply) == FND_OK);
    CHECK(reply.a0 == width && reply.a1 == 30);
    cursor(0, 0);
}
static void bulk_io(void) {
    uint8_t *memory = fnd_mem_alloc(8193 + 4);
    CHECK(memory);
    uint8_t *data = memory + 1; /* unaligned IPC and multiple pages */
    for (uint32_t i = 0; i < 8193; ++i) data[i] = (uint8_t)(i * 37u + 11u);
    fnd_handle file;
    CHECK(fnd_file_open("/dma-copy.bin", FND_OPEN_WRITE | FND_OPEN_CREATE | FND_OPEN_TRUNC,
                        FND_MODE_DEFAULT, &file) == FND_OK);
    uint32_t count;
    CHECK(fnd_write(file, data, 8193, &count) == FND_OK && count == 8193);
    CHECK(fnd_close(file) == FND_OK);
    for (uint32_t i = 0; i < 8193; ++i) data[i] = 0;
    CHECK(fnd_file_open("/dma-copy.bin", FND_OPEN_READ, FND_MODE_DEFAULT, &file) == FND_OK);
    CHECK(fnd_read(file, data, 8193, &count) == FND_OK && count == 8193);
    for (uint32_t i = 0; i < 8193; ++i) CHECK(data[i] == (uint8_t)(i * 37u + 11u));
    CHECK(fnd_read(file, data, 1, &count) == FND_OK && count == 0);
    CHECK(fnd_close(file) == FND_OK);
    for (uint32_t i = 0; i < 4096; ++i) data[i] = i % 80u == 79u ? '\n' : 'D';
    CHECK(fnd_write(fnd_std_out(), data, 4096, &count) == FND_OK && count == 4096);
    out("\033[H\033[2Jbulk console/fs passed\n");
    fnd_mem_free(memory);
}
int main(void) {
    fnd_init();
    if (fnd_argc() > 1) { bulk_io(); return 0; }
    display(FND_CON_DISPLAY_40, 40);
    out("1234567890123456789012345678901234567890"); cursor(0, 1);
    out("\b"); cursor(39, 0);
    out("\t"); cursor(0, 1);
    out("\033[9999;9999H\033[s"); cursor(39, 29);
    uint32_t bad[4] = {40, 0, 0, 0};
    CHECK(fnd_call(fnd_console(), FND_CON_SET_CURSOR, bad, 0, 0, 0, 0, 0) == FND_E_BAD_ARG);
    bad[0] = 2;
    CHECK(fnd_call(fnd_console(), FND_CON_SET_DISPLAY, bad, 0, 0, 0, 0, 0) == FND_E_BAD_ARG);
    cursor(39, 29);
    display(FND_CON_DISPLAY_8X16, 80);
    out("\033[u"); cursor(0, 0);
    for (uint32_t i = 0; i < 46; ++i) out("row\n");
    cursor(0, 29);
    out("\033[H\033[2J8x16\033[K"); cursor(4, 0);
    display(FND_CON_DISPLAY_80, 80);
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
