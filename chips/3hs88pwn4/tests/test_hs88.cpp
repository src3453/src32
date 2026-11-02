// Behavioral tests for the 3HS88PWN4 model that need no reference source.
#include <cstdio>
#include <cstdlib>
#include <vector>

#include "hs88/registers.hpp"

namespace {

int failures = 0;

#define CHECK(cond)                                                              \
    do {                                                                         \
        if (!(cond)) {                                                           \
            std::printf("%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond); \
            failures++;                                                          \
        }                                                                        \
    } while (0)

constexpr std::uint32_t kRegs = 0x400000;

struct Chip {
    hs88::Engine engine;
    hs88::RegisterInterface regs{engine};

    void wt(int ch, int offset, int value) {
        regs.write(kRegs + 0x200 + 48 * ch + offset, static_cast<std::uint8_t>(value));
    }
    std::vector<std::int16_t> render(std::size_t frames) {
        std::vector<std::int16_t> left(frames), right(frames);
        engine.render(left.data(), right.data(), frames);
        return left;
    }
};

// Rising zero crossings: whole cycles of a periodic signal.
int risingCrossings(const std::vector<std::int16_t>& samples) {
    int count = 0;
    for (std::size_t i = 1; i < samples.size(); i++) {
        if (samples[i - 1] < 0 && samples[i] >= 0) count++;
    }
    return count;
}

int peak(const std::vector<std::int16_t>& samples) {
    int value = 0;
    for (std::int16_t s : samples) value = std::max(value, std::abs(static_cast<int>(s)));
    return value;
}

void testAddressSpace() {
    Chip chip;
    chip.regs.write(0x123456, 0x5A);  // PCM RAM
    CHECK(chip.regs.read(0x123456) == 0x5A);
    chip.regs.write(kRegs + 0x3F0, 0x11);  // reserved, stored
    CHECK(chip.regs.read(kRegs + 0x3F0) == 0x11);
    chip.regs.write(kRegs + 0x400, 0x22);  // outside
    CHECK(chip.regs.read(kRegs + 0x400) == 0);
    chip.wt(1, 8, 0x33);  // sync strobe reads 0
    CHECK(chip.regs.read(kRegs + 0x200 + 48 + 8) == 0);
    chip.wt(2, 0x10 + 5, 0x44);  // table byte also feeds wave C
    CHECK(chip.engine.pcm(2).params().table[5] == 0x44);
}

void testSilentAtPowerOn() {
    Chip chip;
    CHECK(peak(chip.render(4800)) == 0);
}

// An undefined mode reads 0, which centers to -128 like the reference.
void testUndefinedModeIsDc() {
    Chip chip;
    chip.wt(0, 2, 255);
    chip.wt(0, 3, 9);
    const std::vector<std::int16_t> out = chip.render(64);
    CHECK(out.front() == out.back());
    CHECK(out.back() < -3000);
}

// The WT cursor restarts on sync: two channels started apart line up again.
void testSync() {
    Chip a, b;
    for (Chip* chip : {&a, &b}) {
        for (int i = 0; i < 32; i++) chip->wt(0, 0x10 + i, i * 8);
        chip->wt(0, 0, 0x01);
        chip->wt(0, 1, 0xB8);
        chip->wt(0, 2, 255);
        chip->wt(0, 3, 1);
    }
    a.render(300);
    b.render(777);
    a.wt(0, 8, 1);
    b.wt(0, 8, 1);
    CHECK(a.render(1024) == b.render(1024));
}

void testFifo() {
    Chip chip;
    chip.wt(0, 2, 255);
    chip.wt(0, 3, 5);
    const std::uint8_t data[3] = {255, 0, 128};
    CHECK(chip.engine.pcm(0).pushFifo(data, 3) == 3);
    const std::vector<std::int16_t> out = chip.render(5);
    CHECK(out[0] > 0 && out[1] < 0 && out[2] == 0);
    CHECK(out[3] == 0 && out[4] == 0);  // empty: holds the last byte
    std::vector<std::uint8_t> big(5000, 1);
    CHECK(chip.engine.pcm(0).pushFifo(big.data(), big.size()) == -2);
    CHECK(chip.engine.pcm(0).fifoLength() == hs88::kFifoCapacity);
}

void testMuteAndPan() {
    Chip chip;
    for (int i = 0; i < 32; i++) chip.wt(1, 0x10 + i, 255);
    chip.wt(1, 2, 255);
    chip.wt(1, 3, 4);
    chip.wt(1, 7, 0x0F);  // right only
    std::vector<std::int16_t> left(256), right(256);
    chip.engine.render(left.data(), right.data(), 256);
    CHECK(peak(left) == 0);
    CHECK(peak(right) > 3000);
    chip.regs.write(kRegs + 0x2D0 + 9, 1);
    chip.engine.render(left.data(), right.data(), 256);
    CHECK(peak(right) == 0);
}

// A loud low-passed square stays finite and the filter can be bypassed.
void testFilterBounds() {
    Chip chip;
    for (int i = 0; i < 32; i++) chip.wt(0, 0x10 + i, i < 16 ? 255 : 0);
    chip.wt(0, 0, 0);
    chip.wt(0, 1, 100);
    chip.wt(0, 2, 255);
    chip.wt(0, 3, 4);
    for (int mode = 0; mode < 6; mode++) {
        chip.wt(0, 4, mode);
        chip.wt(0, 5, 255);
        chip.wt(0, 6, 255);
        const std::vector<std::int16_t> out = chip.render(4800);
        CHECK(peak(out) <= 32767);
    }
}

// WT pitch is exact Hz: the reference's floor(192000 / floor(192000 / F))
// would play 7000 Hz as 7111 Hz.
void testWaveFrequencyIsExact() {
    for (int hz : {7000, 1001, 45}) {
        Chip chip;
        for (int i = 0; i < 32; i++) chip.wt(0, 0x10 + i, i < 16 ? 255 : 0);
        chip.wt(0, 0, hz >> 8);
        chip.wt(0, 1, hz & 0xFF);
        chip.wt(0, 2, 255);
        chip.wt(0, 3, 4);
        const int cycles = risingCrossings(chip.render(hs88::kSampleRate));
        CHECK(std::abs(cycles - hz) <= 1);
    }
}

}  // namespace

int main() {
    testAddressSpace();
    testSilentAtPowerOn();
    testUndefinedModeIsDc();
    testSync();
    testFifo();
    testMuteAndPan();
    testFilterBounds();
    testWaveFrequencyIsExact();
    if (failures == 0) std::printf("all 3HS88PWN4 tests passed\n");
    return failures == 0 ? 0 : 1;
}
