#include "counter.h"

// The SDK sources are compiled as C; retain their unmangled symbol names.
extern "C" {
#include <cpt32/pcg.h>
}

// Freestanding Clang also mangles main unless its C linkage is explicit.
extern "C" int main() {
    static_assert(sizeof(void *) == 4, "SRC32 uses 32-bit pointers");
    static_assert(sizeof(long) == 4, "SRC32 uses 32-bit long");

    pcg_initPCG();
    pcg_cls(0);
    pcg_colorfg(0x3f);
    pcg_colorbg(0);
    pcg_prnlnscr("Hello C++17 + LLVM / SRC32!");

    demo::Counter counter{7};
    uint32_t result = 0;
    for (uint32_t i = 0; i < 7; ++i)
        result = counter.advance(5);

    if (result != demo::expected)
        return 1;
    pcg_prnlnscr("Counter: 7 + 7 * 5 = 42");

    // Check text VRAM as well as the class calculation on the real guest CPU.
    const volatile uint8_t *text =
        reinterpret_cast<const volatile uint8_t *>(pcg_vram_base);
    if (text[0] != 'H' || text[40] != 'C')
        return 2;
    return static_cast<int>(result);
}
