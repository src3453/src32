"""Create an original CHIP-8 keypad/font/beep demo (no third-party ROM)."""
import argparse
from pathlib import Path

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    # Clear; wait for key+release; show its hex digit at (30,14); beep; repeat.
    ops = [0x00E0, 0xF00A, 0x00E0, 0xF029, 0x611E, 0x620E, 0xD125,
           0x6308, 0xF318, 0x1202]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(b''.join(op.to_bytes(2, 'big') for op in ops))
    print(args.output)

if __name__ == '__main__':
    main()
