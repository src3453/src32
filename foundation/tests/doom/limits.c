#include <foundation/fnd.h>
#include <foundation/sys.h>

#define CHECK(x) do { if (!(x)) fnd_exit(__LINE__); } while (0)
_Static_assert(FND_EXEC_IMAGE_MAX == FND_SPAWN_IMAGE_MAX, "exec/spawn limits differ");

void main(void) {
    fnd_init();
    fnd_spawn sp = {0};
    sp.priority = FND_PRIO_APP;
    sp.image_len = FND_SPAWN_IMAGE_MAX + 1u;
    CHECK(fnd_sys_proc_spawn(&sp) == FND_E_LIMIT);
    void *image = fnd_mem_alloc(FND_SPAWN_IMAGE_MAX);
    CHECK(image);
    *(uint32_t *)image = 0; /* deliberately invalid WAX after capacity checks */
    sp.image = (uint32_t)(uintptr_t)image;
    sp.name = (uint32_t)(uintptr_t)"limits";
    uint32_t libs[] = {1, 0, sp.image, 4};
    sp.libs = (uint32_t)(uintptr_t)libs;
    sp.image_len = FND_SPAWN_IMAGE_MAX;
    CHECK(fnd_sys_proc_spawn(&sp) == FND_E_LIMIT); /* WAX fits, WAL doesn't */
    sp.image_len = FND_SPAWN_IMAGE_MAX - 1;
    CHECK(fnd_sys_proc_spawn(&sp) == FND_E_LIMIT); /* alignment uses the last byte */
    sp.image_len = FND_SPAWN_IMAGE_MAX - 4;
    CHECK(fnd_sys_proc_spawn(&sp) == FND_E_FORMAT); /* exact total fits */
    fnd_mem_free(image);
    fnd_con_write("doom spawn limits passed\n");
    fnd_exit(0);
}
