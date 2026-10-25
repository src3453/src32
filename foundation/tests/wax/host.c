/* Host shim for test_wax_loader.py: runs the loader core against a fake RAM
 * buffer that stands for addresses [ram_base, ram_base + its size). */
#include "../../kernel/wax.h"

static struct wax_launch launch;
static uint32_t load_base; /* host_set_load_base: where a relocatable executable goes */

void host_set_load_base(uint32_t base) { load_base = base; }

/* out: error, count, entry, stack_top, stack_size, top, region_count,
 * regions (start, end, flags) x 8, origins x 8, diag_image, diag_index. */
int host_plan(const uint8_t *exe, uint32_t size, const uint8_t *const *libs, const uint32_t *lib_sizes,
              uint32_t lib_count, uint32_t window_start, uint32_t window_end, uint8_t *ram,
              uint32_t ram_base, uint32_t *out) {
    struct wax_library table[16];
    struct wax_target target = {"foundation", 1, 0, window_start, window_end, load_base};
    for (uint32_t i = 0; i < lib_count && i < 16u; ++i) {
        table[i].data = libs[i];
        table[i].size = lib_sizes[i];
    }
    enum wax_error error = wax_plan(&launch, exe, size, table, lib_count, &target);
    uint32_t n = 0;
    out[n++] = error;
    out[n++] = launch.count;
    out[n++] = launch.entry;
    out[n++] = launch.stack_top;
    out[n++] = launch.stack_size;
    out[n++] = launch.top;
    out[n++] = launch.region_count;
    for (uint32_t i = 0; i < WAX_REGIONS; ++i) {
        out[n++] = launch.region[i].start;
        out[n++] = launch.region[i].end;
        out[n++] = launch.region[i].flags;
    }
    for (uint32_t i = 0; i < WAX_MAX_IMAGES; ++i)
        out[n++] = launch.image[i].origin;
    out[n++] = launch.diag_image;
    out[n++] = launch.diag_index;
    if (error == WAX_OK && ram)
        wax_load(&launch, (uintptr_t)ram - ram_base);
    return (int)error;
}

const char *host_diag_name(void) { return launch.diag_name; }

int host_base_addr(const uint8_t *data, uint32_t size, uint32_t *base) {
    return (int)wax_base_addr(data, size, base);
}

/* out: error, base, low, high, relocatable, stack_size, heap_hint. */
int host_inspect(const uint8_t *data, uint32_t size, uint32_t *out) {
    struct wax_info info = {0};
    enum wax_error error = wax_inspect(data, size, &info);
    out[0] = error;
    out[1] = info.base;
    out[2] = info.low;
    out[3] = info.high;
    out[4] = info.relocatable;
    out[5] = info.stack_size;
    out[6] = info.heap_hint;
    return (int)error;
}
