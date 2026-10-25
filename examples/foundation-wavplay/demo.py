"""Generate a stereo PCM16 WAV: 440 Hz left, 660 Hz right (no dependencies)."""
import argparse
import math
from pathlib import Path
import struct
import wave


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    rate = 44100
    with wave.open(str(args.output), 'wb') as out:
        out.setparams((2, 2, rate, 0, 'NONE', 'not compressed'))
        for block in range(3 * rate // 441):
            data = bytearray()
            for i in range(block * 441, (block + 1) * 441):
                # Short fades reduce the start/end discontinuity.
                gain = min(1, i / 441, (3 * rate - 1 - i) / 441) * 10000
                data += struct.pack('<hh', *(int(gain * math.sin(2 * math.pi * hz * i / rate))
                                             for hz in (440, 660)))
            out.writeframesraw(data)


if __name__ == '__main__':
    main()
