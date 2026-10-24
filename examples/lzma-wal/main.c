/* Foundation external-library demo: these calls import lzma ABI 1.1. */
#include <foundation/lzma.h>
#include <string.h>

static uint8_t packed[256], restored[256];
static const uint8_t message[] =
    "Foundation LZMA WAL demo. Foundation LZMA WAL demo. Foundation LZMA WAL demo.";

int main(void) {
    uint64_t workspace[262144]; /* 2 MiB, 8-byte aligned; caller's stack. */
    uint32_t packed_size = sizeof(packed), restored_size = sizeof(restored);
    if (cpt_lzma_compress(packed, &packed_size, message, sizeof(message),
                          workspace, sizeof(workspace), 1, 4096))
        return 101;
    if (packed_size >= sizeof(message))
        return 102;
    if (cpt_lzma_decompress(restored, &restored_size, packed, packed_size,
                            workspace, sizeof(workspace)))
        return 103;
    if (restored_size != sizeof(message) || memcmp(message, restored, sizeof(message)))
        return 104;
    return 42;
}
