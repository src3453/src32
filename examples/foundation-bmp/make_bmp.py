"""Writes sample BMPs for /bin/bmpview: python make_bmp.py OUT.bmp [8|24]."""
import struct
import sys


def make_bmp(width=160, height=120, bpp=24):
    """A bottom-up, uncompressed BMP: a diagonal red/green/blue gradient with a white frame."""
    def colour(x, y):
        if x in (0, width - 1) or y in (0, height - 1):
            return (255, 255, 255)
        return (x * 255 // (width - 1), y * 255 // (height - 1), 255 - (x + y) * 255 // (width + height - 2))

    stride = (width * bpp // 8 + 3) & ~3
    palette = b''
    if bpp == 8:  # 3-3-2 palette (B, G, R, 0 per entry); the pixel is the nearest entry
        palette = b''.join(bytes([(i >> 5) * 255 // 7, ((i >> 2) & 7) * 255 // 7, (i & 3) * 85, 0]) for i in range(256))
    rows = []
    for y in range(height - 1, -1, -1):
        row = bytearray()
        for x in range(width):
            r, g, b = colour(x, y)
            if bpp == 8:
                row.append((r >> 6) | ((g >> 5) << 2) | ((b >> 5) << 5))
            else:
                row += bytes((b, g, r))
        rows.append(bytes(row).ljust(stride, b'\0'))
    pixels = b''.join(rows)
    offset = 14 + 40 + len(palette)
    return (b'BM' + struct.pack('<IHHI', offset + len(pixels), 0, 0, offset) +
            struct.pack('<IiiHHIIiiII', 40, width, height, 1, bpp, 0, len(pixels), 2835, 2835,
                        256 if bpp == 8 else 0, 0) + palette + pixels)


if __name__ == '__main__':
    open(sys.argv[1], 'wb').write(make_bmp(bpp=int(sys.argv[2]) if len(sys.argv) > 2 else 24))
