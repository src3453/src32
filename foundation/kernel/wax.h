#ifndef FOUNDATION_KERNEL_WAX_H
#define FOUNDATION_KERNEL_WAX_H
#include <stdint.h>

/* WAX loader core (doc/spec_WAX_v3.md, doc/spec_WAX.md "Protected
 * executables", doc/spec_Foundation.md §5, §9.2). It validates a PROTECTED
 * executable and, for v3, resolves its WAL graph, places the libraries,
 * plans the protection regions and binds the import slots. It touches no
 * kernel state and no device, so it also builds and runs on the host
 * (foundation/tests/wax). */

/* Resource limits applied before any allocation (v3 §7 step 1). */
#define WAX_MAX_IMAGES 8u  /* executable + libraries per launch */
#define WAX_MAX_BLOCKS 32u /* per image */
#define WAX_MAX_DEPS 8u    /* dependencies per image */
#define WAX_REGIONS 8u     /* Extension P regions per context */

/* Logical error categories (v3 §7). The kernel maps them to FND_E_*. */
enum wax_error {
    WAX_OK,
    WAX_E_FORMAT,
    WAX_E_UNSUPPORTED,
    WAX_E_DEPENDENCY_MISSING,
    WAX_E_ABI_MISMATCH,
    WAX_E_DEPENDENCY_CONFLICT,
    WAX_E_DEPENDENCY_CYCLE,
    WAX_E_SYMBOL_MISSING,
    WAX_E_SYMBOL_MISMATCH,
    WAX_E_REGION_LIMIT,
    WAX_E_RESOURCE_LIMIT,
};

/* One parsed image. Offsets are file offsets unless noted. */
struct wax_image {
    const uint8_t *data;
    uint32_t size;
    uint32_t version; /* 2 or 3 */
    uint32_t type;    /* 1 WAX, 2 WAL */
    uint32_t base;    /* WAX base_addr; 0 for a WAL */
    uint32_t origin;  /* runtime origin: base_addr, or the WAL's base B */
    uint32_t span;    /* reservation: highest block end - lowest address */
    uint32_t align;   /* largest block alignment */
    uint32_t entry_offset, stack_top, stack_size;
    uint32_t block_count, block_table, block_entry_size;
    uint32_t dep_count, dep_table;
    uint32_t export_count, export_table;
    uint32_t import_count, import_table;
    uint32_t strings, string_size;
    uint32_t image_id, abi_major, abi_minor; /* string offsets are table-relative */
    uint8_t dep_image[WAX_MAX_DEPS];         /* launch index chosen for each dependency */
};

struct wax_region {
    uint32_t start, end, flags;
};

/* An installed library the loader may select as a dependency provider. */
struct wax_library {
    const void *data;
    uint32_t size;
};

/* What the OS supports and where the process may live. */
struct wax_target {
    const char *os_id;
    uint32_t os_major, os_minor;
    uint32_t window_start, window_end; /* all blocks, libraries and the stack */
};

struct wax_launch {
    uint32_t count;                         /* image[0] is the executable */
    struct wax_image image[WAX_MAX_IMAGES]; /* libraries in placement order */
    uint32_t entry, stack_top, stack_size;
    uint32_t top; /* end of the highest block: where the heap starts (§5.3) */
    uint32_t region_count;
    struct wax_region region[WAX_REGIONS]; /* [0] is the stack */
    /* Diagnostics for a failed plan: the requesting image (launch index or,
     * for a rejected library candidate, WAX_MAX_IMAGES), its dependency or
     * import index, and the symbol or library ID involved (NUL-terminated
     * inside its image, or 0). */
    uint32_t diag_image, diag_index;
    const char *diag_name;
};

/* Reads base_addr of a v2 or v3 executable after checking the magic and
 * version, so the caller can pick the window before wax_plan. */
enum wax_error wax_base_addr(const void *data, uint32_t size, uint32_t *base);

/* Validates the executable and every library it needs, selects providers,
 * places the libraries inside the window, plans the regions and checks every
 * import. Writes nothing outside *launch. */
enum wax_error wax_plan(struct wax_launch *launch, const void *data, uint32_t size,
                        const struct wax_library *libs, uint32_t lib_count,
                        const struct wax_target *target);

/* Copies the blocks of a successful plan, zeroes their tails and the stack,
 * and writes the import descriptors. Address a is written at bias + a. */
void wax_load(const struct wax_launch *launch, uintptr_t bias);

/* The same through a mapping, for an address space the loader cannot write
 * directly: map(context, a, &run) returns where process address a lives and
 * how many bytes from there are contiguous (at least 1). */
typedef uint8_t *(*wax_map_fn)(void *context, uint32_t address, uint32_t *run);
void wax_load_mapped(const struct wax_launch *launch, wax_map_fn map, void *context);

#endif
