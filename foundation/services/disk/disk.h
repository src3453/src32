#ifndef FOUNDATION_SERVICES_DISK_H
#define FOUNDATION_SERVICES_DISK_H

/* The protocol between the fs service and the disk service ("disk"). The
 * API spec leaves it to the implementation (spec_Foundation_API.md §1), so
 * it uses service-specific operations (§2.3). Sectors are 512 bytes; one
 * call moves at most FND_IPC_MAX_DATA bytes.
 *
 *   DISK_IDENTIFY  a0 = drive                          a0..a3 = type, sector_size, capacity, flags
 *   DISK_READ      a0 = drive, a1 = lba, a2 = count    data = count sectors
 *   DISK_WRITE     a0 = drive, a1 = lba, data = sectors  a0 = sectors written
 *
 * Results are the blk_io results (spec_Foundation.md §10.4). The disk
 * service serves only its first client, the fs service, which the boot
 * image starts right after it: v1 has no way to check who calls
 * (spec_Foundation_API.md §9, §14), and raw sectors must not reach other
 * processes (spec_FS2.md §11.1). */

#define DISK_IDENTIFY 0x8000u
#define DISK_READ 0x8001u
#define DISK_WRITE 0x8002u
#define DISK_MAX_SECTORS 8u

/* IDENTIFY flags (spec_IDC.md). */
#define DISK_FLAG_PRESENT 1u
#define DISK_FLAG_READ_ONLY 2u

#endif
