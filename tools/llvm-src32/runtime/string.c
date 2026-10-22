#include <stdint.h>
#include <string.h>

/* Word accesses require alignment and must alias arbitrary C objects. */
typedef uint32_t alias_word __attribute__((__may_alias__));

/* The driver omits each definition already provided by the user's modules. */
#ifndef SRC32_HAVE_memcpy
void *memcpy(void *restrict destination, const void *restrict source, size_t count) {
    unsigned char *out = destination;
    const unsigned char *in = source;
    if ((((uintptr_t)out ^ (uintptr_t)in) & 3u) == 0) {
        while (count && ((uintptr_t)out & 3u)) {
            *out++ = *in++;
            --count;
        }
        while (count >= 4) {
            *(alias_word *)(void *)out = *(const alias_word *)(const void *)in;
            out += 4;
            in += 4;
            count -= 4;
        }
    }
    while (count--) *out++ = *in++;
    return destination;
}
#endif

#ifndef SRC32_HAVE_memmove
void *memmove(void *destination, const void *source, size_t count) {
    unsigned char *out = destination;
    const unsigned char *in = source;
    /* Integer addresses make ordering unrelated objects well-defined here. */
    if ((uintptr_t)out < (uintptr_t)in) {
        for (size_t i = 0; i < count; ++i)
            out[i] = in[i];
    } else {
        while (count != 0) {
            --count;
            out[count] = in[count];
        }
    }
    return destination;
}
#endif

#ifndef SRC32_HAVE_memset
void *memset(void *destination, int value, size_t count) {
    unsigned char *out = destination;
    unsigned char byte = (unsigned char)value;
    while (count && ((uintptr_t)out & 3u)) {
        *out++ = byte;
        --count;
    }
    uint32_t word = (uint32_t)byte * 0x01010101u;
    while (count >= 4) {
        *(alias_word *)(void *)out = word;
        out += 4;
        count -= 4;
    }
    while (count--) *out++ = byte;
    return destination;
}
#endif

#ifndef SRC32_HAVE_memcmp
int memcmp(const void *left, const void *right, size_t count) {
    const unsigned char *a = left;
    const unsigned char *b = right;
    for (size_t i = 0; i < count; ++i) {
        if (a[i] != b[i])
            return (int)a[i] - (int)b[i];
    }
    return 0;
}
#endif

#ifndef SRC32_HAVE_strlen
size_t strlen(const char *text) {
    size_t length = 0;
    while (text[length] != '\0')
        ++length;
    return length;
}
#endif
