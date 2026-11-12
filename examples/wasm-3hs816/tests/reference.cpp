// Render tests/script.txt with the C++ 3HS816 model (chips/3hs816) and write
// interleaved 16-bit little-endian stereo to the output file. Regenerates
// tests/reference.pcm; see README.md.
#include <cstdint>
#include <cstdio>
#include <fstream>
#include <string>
#include <vector>

#include "hs816/registers.hpp"

int main(int argc, char** argv) {
    if (argc != 3) {
        std::fprintf(stderr, "usage: reference SCRIPT OUT.pcm\n");
        return 2;
    }
    std::ifstream script(argv[1]);
    hs816::Engine engine;
    hs816::RegisterInterface regs(engine);
    std::vector<std::uint8_t> out;
    std::string op;
    while (script >> op) {
        if (op == "w") {
            unsigned address = 0, value = 0;
            script >> std::hex >> address >> value >> std::dec;
            regs.write(address, static_cast<std::uint8_t>(value));
        } else if (op == "r") {
            std::size_t frames = 0;
            script >> frames;
            std::vector<std::int16_t> left(frames), right(frames);
            engine.render(left.data(), right.data(), frames);
            for (std::size_t i = 0; i < frames; i++) {
                for (std::int16_t s : {left[i], right[i]}) {
                    out.push_back(static_cast<std::uint8_t>(s));
                    out.push_back(static_cast<std::uint8_t>(static_cast<std::uint16_t>(s) >> 8));
                }
            }
        } else {
            std::fprintf(stderr, "bad script op %s\n", op.c_str());
            return 1;
        }
    }
    std::ofstream(argv[2], std::ios::binary).write(reinterpret_cast<const char*>(out.data()), out.size());
    return 0;
}
