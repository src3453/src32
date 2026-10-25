/* RAM-backed WAD class: avoid repeated random walks over an FS2 block chain.
 * Data is loaded from the user's disk at runtime, never linked into the WAX. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "w_file.h"

extern wad_file_class_t stdc_wad_file;

static wad_file_t *open_wad(char *path) {
    FILE *file = fopen(path, "rb");
    if (!file) return NULL;
    wad_file_t *wad = NULL;
    long size;
    if (fseek(file, 0, SEEK_END) || (size = ftell(file)) < 0 || size > 24 * 1024 * 1024 ||
        fseek(file, 0, SEEK_SET)) goto fail;
    wad = calloc(1, sizeof *wad);
    if (!wad) goto fail;
    wad->mapped = malloc(size ? (size_t)size : 1u);
    if (!wad->mapped) goto fail;
    if (fread(wad->mapped, 1, (size_t)size, file) != (size_t)size) goto fail;
    wad->length = (unsigned)size;
    wad->file_class = &stdc_wad_file;
    fclose(file);
    return wad;
fail:
    if (wad) { free(wad->mapped); free(wad); }
    fclose(file);
    return NULL;
}

static void close_wad(wad_file_t *wad) {
    free(wad->mapped);
    free(wad);
}

static size_t read_wad(wad_file_t *wad, unsigned offset, void *buffer, size_t bytes) {
    if (offset > wad->length) return 0;
    if (bytes > wad->length - offset) bytes = wad->length - offset;
    memcpy(buffer, wad->mapped + offset, bytes);
    return bytes;
}

wad_file_class_t stdc_wad_file = {open_wad, close_wad, read_wad};
