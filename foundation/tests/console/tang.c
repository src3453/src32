/* Real Tang console backend; fail on any access to absent MMIO/VRAM. */
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
static uint8_t vram[0x22000], vdp[0x10000];
static uint8_t *mmio8(uint32_t a) {
    if (a >= 0x10000000u && a < 0x10022000u) {
        uint32_t n = a - 0x10000000u;
        assert(n < 0x3000 || (n >= 0x12c00 && n < 0x12cc0) || n >= 0x20000);
        return &vram[n];
    }
    assert(a >= 0x80000000u && a < 0x80010000u);
    return &vdp[a - 0x80000000u];
}
#define MMIO8(a) (*mmio8(a))
#define FND_CON_TANG
#include "../../services/console/con_tang.c"
#include "../../services/console/con_pcg.c"

static void save(const char *dir, unsigned n) {
    char name[1024];
    for (unsigned k = 0; k < 2; ++k) {
        snprintf(name, sizeof name, "%s/case_%u.%s.bin", dir, n, k ? "regs" : "vram");
        FILE *f = fopen(name, "wb"); assert(f);
        assert(fwrite(k ? vdp : vram, 1, k ? sizeof vdp : sizeof vram, f) ==
               (k ? sizeof vdp : sizeof vram));
        assert(!fclose(f));
    }
}

int main(int argc, char **argv) {
    assert(con_pcg_init() == FND_OK);
    assert(!memcmp(vram + 0x20000, tang_fonts, sizeof tang_fonts));
    assert(!memcmp(vram + 0x12c00, tang_palette, sizeof tang_palette));
    assert(vdp[1] == 1 && vdp[0xf008] == 0xfe);
    fnd_key_t key;
    assert(!con_pcg_poll_key(&key)); /* no PeC input MMIO access */
    const unsigned modes[] = {FND_CON_DISPLAY_80, FND_CON_DISPLAY_40, FND_CON_DISPLAY_8X16};
    for (unsigned n = 0; n < 3; ++n) {
        assert(con_pcg_set_display(modes[n]) == FND_OK);
        for (unsigned row = 0; row < 37; ++row)
            con_pcg_write((const uint8_t *)"scroll check\n", 13);
        const char *s = "\033[2;3H\033[31;44mFoundation VDP Console\033[0m\n"
                        "ANSI color / scroll / 40,80,8x16\n";
        con_pcg_write((const uint8_t *)s, (uint32_t)strlen(s));
        con_pcg_write((const uint8_t *)"\xe6\x97\xa5", 3); /* '?' fallback, no wide VRAM */
        vdp[0xf007] = 0; vdp[0xf009] = 0; /* stable golden frame */
        if (argc == 2) save(argv[1], n);
    }
    assert(con_pcg_set_display(2) == FND_E_BAD_ARG);
    puts("Tang PCG console passed");
    return 0;
}
