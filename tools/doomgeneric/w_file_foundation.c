/* WAD class with two modes. Data is loaded from the user's disk at runtime,
 * never linked into the WAX.
 *  - RAM-backed: the WAD is read once and served from memory. This avoids
 *    repeated random walks over an FS2 block chain, but needs the whole WAD
 *    in RAM.
 *  - Streamed: for WADs larger than WAD_RAM_LIMIT (16 MiB boards) the file
 *    stays open and each lump read is an fseek + fread. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#ifdef _WIN32 /* host unit tests */
#include <io.h>
#define open _open
#define lseek _lseek
#define read _read
#define close _close
#define O_RDONLY (_O_RDONLY | _O_BINARY)
typedef int ssize_t;
typedef long off_t;
#else
#include <unistd.h>
#endif
#include "w_file.h"

#ifndef WAD_RAM_LIMIT
#define WAD_RAM_LIMIT (5 * 1024 * 1024)
#endif

extern wad_file_class_t stdc_wad_file;
extern unsigned DG_GetTicksMs(void);

/* Read statistics for doomgeneric_foundation.c's report. */
unsigned wad_reads, wad_bytes, wad_ms;
/* WADs above this size are streamed; a variable so the host test can use both modes. */
unsigned wad_ram_limit = WAD_RAM_LIMIT;

typedef struct {
    wad_file_t wad;
    int fd;
} stream_wad_t;

static wad_file_t *open_wad(char *path) {
    FILE *file = fopen(path, "rb");
    if (!file) return NULL;
    wad_file_t *wad = NULL;
    long size;
    if (fseek(file, 0, SEEK_END) || (size = ftell(file)) < 0 || size > 24 * 1024 * 1024 ||
        fseek(file, 0, SEEK_SET)) goto fail;
    if (size > (long)wad_ram_limit) {
        /* Plain open/lseek/read: stdio would add buffering copies and several
         * IPC round trips to the file service per lump. */
        int fd = open(path, O_RDONLY);
        if (fd < 0) goto fail;
        stream_wad_t *stream = calloc(1, sizeof *stream);
        if (!stream) { close(fd); goto fail; }
        stream->fd = fd;
        stream->wad.length = (unsigned)size;
        stream->wad.file_class = &stdc_wad_file;
        fclose(file);
        return &stream->wad;
    }
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
    if (!wad->mapped) {
        close(((stream_wad_t *)wad)->fd);
    } else {
        free(wad->mapped);
    }
    free(wad);
}

static size_t read_wad(wad_file_t *wad, unsigned offset, void *buffer, size_t bytes) {
    if (offset > wad->length) return 0;
    if (bytes > wad->length - offset) bytes = wad->length - offset;
    if (!wad->mapped) {
        int fd = ((stream_wad_t *)wad)->fd;
        unsigned started = DG_GetTicksMs();
        size_t got = 0;
        if (lseek(fd, (off_t)offset, SEEK_SET) == (off_t)offset) {
            while (got < bytes) {
                ssize_t n = read(fd, (char *)buffer + got, bytes - got);
                if (n <= 0) break;
                got += (size_t)n;
            }
        }
        ++wad_reads, wad_bytes += (unsigned)got, wad_ms += DG_GetTicksMs() - started;
        return got;
    }
    memcpy(buffer, wad->mapped + offset, bytes);
    return bytes;
}

wad_file_class_t stdc_wad_file = {open_wad, close_wad, read_wad};
