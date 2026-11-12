"""Write tests/script.txt: a deterministic 3HS816 register script.

Lines are `w ADDRESS VALUE` (hex) or `r FRAMES` (decimal). Both the C++
reference harness (reference.cpp) and the Rust test replay it.
"""
import pathlib
import random

OUT = pathlib.Path(__file__).resolve().parent / 'script.txt'


def main():
    rng = random.Random(3816)
    lines = []

    def w(address, value):
        lines.append(f'w {address:03x} {value & 0xff:02x}')

    def r(frames):
        lines.append(f'r {frames}')

    # User waves A..D.
    for wave in range(4):
        for i in range(32):
            w(0x400 + wave * 32 + i, rng.randrange(256))
    for ch in range(16):
        w(0x490 + ch, ch % 9)  # 8 disables feedback
    for round_ in range(6):
        for ch in range(16):
            base = ch * 0x40
            freq = rng.choice([55, 110, 220, 440, 750, 1000, 1234, 3000, 65535, 0, 37])
            w(base + 0, freq >> 8)
            w(base + 1, freq)
            for op in range(1, 8):
                ratio = rng.choice([4096, 8192, 2048, 4096 * 3, 6144, rng.randrange(65536)])
                w(base + 2 * op, ratio >> 8)
                w(base + 2 * op + 1, ratio)
            for op in range(8):
                w(base + 0x10 + op, rng.choice([0, 64, 128, 255, rng.randrange(256)]))
            for pair in range(4):
                w(base + 0x18 + pair, rng.randrange(256))
            w(base + 0x1c, (round_ * 16 + ch) % 15)  # 13, 14 are silent
            w(base + 0x1d, rng.choice([0x00, 0xf0, 0x0f, 0x8c, rng.randrange(256)]))
            w(base + 0x1f, rng.choice([128, 0, 255, 160, rng.randrange(256)]))
            for op in range(8):
                for i, choices in enumerate(([0, 1, 4, 30], [0, 1, 3, 20], [0, 128, 255, 200], [0, 1, 2, 10])):
                    w(base + 0x20 + op * 4 + i, rng.choice(choices))
            w(0x480 + ch, 1 if rng.random() < 0.1 else 0)
            w(base + 0x1e, 1)
        r(1500)
        # Mid-note changes, retrigger and release.
        for ch in range(0, 16, 3):
            base = ch * 0x40
            w(base + 0x1, rng.randrange(256))
            w(base + 0x21, rng.choice([0, 2, 50]))
        r(700)
        for ch in range(16):
            w(ch * 0x40 + 0x1e, 1 if ch % 4 == 0 else 0)
        r(1800)
        for ch in range(1, 16, 2):
            w(ch * 0x40 + 0x1e, 0)
            w(ch * 0x40 + 0x1e, 0)
        r(1000)
    OUT.write_text('\n'.join(lines) + '\n', encoding='utf-8', newline='\n')


if __name__ == '__main__':
    main()
