// Behavioral tests for 3HS816 that do not need the 3HS88PWN4 reference.
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <vector>

#include "hs816/registers.hpp"

namespace {

int failures = 0;

#define CHECK(cond)                                                       \
    do {                                                                  \
        if (!(cond)) {                                                    \
            std::printf("%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond); \
            failures++;                                                   \
        }                                                                 \
    } while (0)

struct Chip {
    hs816::Engine engine;
    hs816::RegisterInterface regs{engine};

    void write16(std::uint32_t address, int value) {
        regs.write(address, static_cast<std::uint8_t>(value >> 8));
        regs.write(address + 1, static_cast<std::uint8_t>(value & 0xFF));
    }

    // OP1-only sine at full volume with an instant attack and full sustain.
    void sine(int ch, int hz, int pan = 0) {
        const std::uint32_t base = 0x40u * static_cast<std::uint32_t>(ch);
        write16(base, hz);
        regs.write(base + 0x10, 255);
        regs.write(base + 0x22, 255);
        regs.write(base + 0x1D, static_cast<std::uint8_t>(pan));
        regs.write(base + 0x1F, 128);
        regs.write(base + 0x1E, 1);
    }

    std::vector<std::int16_t> render(std::size_t frames, std::vector<std::int16_t>* right = nullptr) {
        std::vector<std::int16_t> left(frames), r(frames);
        engine.render(left.data(), r.data(), frames);
        if (right) *right = r;
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

void testIdAndAddressSpace() {
    Chip chip;
    const char id[] = "3HS816";
    for (int i = 0; i < 6; i++) CHECK(chip.regs.read(0x4F8 + i) == static_cast<std::uint8_t>(id[i]));
    CHECK(chip.regs.read(0x4FE) == 1);
    CHECK(chip.regs.read(0x4FF) == 0);

    chip.regs.write(0x4F8, 0);  // read-only
    CHECK(chip.regs.read(0x4F8) == '3');

    chip.regs.write(0x500, 0x55);  // outside the chip
    CHECK(chip.regs.read(0x500) == 0);

    chip.regs.write(0x3FF, 0xA5);  // channel 15 envelope byte
    CHECK(chip.regs.read(0x3FF) == 0xA5);
    chip.regs.write(0x4C0, 0x5A);  // reserved, stored
    CHECK(chip.regs.read(0x4C0) == 0x5A);
}

void testDecoding() {
    Chip chip;
    chip.write16(0x40 * 3 + 0x00, 0x1234);
    chip.write16(0x40 * 3 + 0x0E, 0x2345);
    chip.regs.write(0x40 * 3 + 0x1A, 0xC7);
    chip.regs.write(0x40 * 3 + 0x1D, 0x9E);
    chip.regs.write(0x40 * 3 + 0x3D, 77);
    chip.regs.write(0x490 + 3, 6);
    chip.regs.write(0x480 + 3, 2);
    const hs816::ChannelParams& p = chip.engine.channel(3).params();
    CHECK(p.frequency == 0x1234);
    CHECK(p.operators[7].ratio == 0x2345);
    CHECK(p.operators[4].wave == 0xC);
    CHECK(p.operators[5].wave == 0x7);
    CHECK(p.panLeft == 0x9 && p.panRight == 0xE);
    CHECK(p.operators[7].envelope.decay == 77);
    CHECK(p.feedbackTarget == 6);
    CHECK(p.muted);

    chip.regs.reset();
    CHECK(chip.engine.channel(3).params().frequency == 0);
    CHECK(chip.regs.read(0x40 * 3) == 0);
}

void testSilentAtPowerOn() {
    Chip chip;
    CHECK(peak(chip.render(4800)) == 0);
}

void testGate() {
    Chip chip;
    chip.sine(0, 440);
    // Release of 4 -> 0.09375 s.
    chip.regs.write(0x23, 4);
    CHECK(chip.engine.channel(0).gateOn());
    CHECK(peak(chip.render(2400)) > 3000);

    chip.regs.write(0x1E, 0);
    CHECK(!chip.engine.channel(0).gateOn());
    chip.render(4800);  // release done after 0.1 s
    CHECK(peak(chip.render(2400)) == 0);

    chip.regs.write(0x1E, 7);  // stored only
    CHECK(chip.regs.read(0x1E) == 7);
    CHECK(peak(chip.render(2400)) == 0);

    chip.regs.write(0x1E, 1);
    CHECK(peak(chip.render(2400)) > 3000);
}

// A retrigger restarts the phase: the output after two key-ons at different
// times is identical.
void testRetriggerResetsPhase() {
    Chip a, b;
    a.sine(0, 333);
    b.sine(0, 333);
    a.render(1000);
    b.render(1777);
    a.regs.write(0x1E, 1);
    b.regs.write(0x1E, 1);
    CHECK(a.render(2048) == b.render(2048));
}

// 750 Hz is exactly 64 samples per cycle and 2^26 per sample in Q32. Long
// notes must stay periodic: the 3HS88PWN4 accumulated float phase without
// bound and lost precision within seconds.
void testLongNoteStaysPeriodic() {
    Chip chip;
    chip.sine(0, 750);
    chip.render(64 * 10);
    const std::vector<std::int16_t> early = chip.render(64);
    for (int second = 0; second < 60; second++) {
        chip.render(hs816::kSampleRate);
    }
    CHECK(chip.render(64) == early);
}

void testPanMuteAndChannelEquivalence() {
    Chip chip;
    chip.sine(15, 500, 0xF0);  // left only
    std::vector<std::int16_t> right;
    std::vector<std::int16_t> left = chip.render(2400, &right);
    CHECK(peak(left) > 3000);
    CHECK(peak(right) == 0);

    chip.regs.write(0x480 + 15, 1);
    left = chip.render(2400, &right);
    CHECK(peak(left) == 0);

    // Channel 0 and channel 15 with the same settings sound the same.
    Chip low, high;
    low.sine(0, 500);
    high.sine(15, 500);
    CHECK(low.render(4096) == high.render(4096));
}

void testUserWave() {
    Chip chip;
    // A constant user wave of 255 is +127 everywhere.
    for (int i = 0; i < 32; i++) chip.regs.write(0x400 + 32 * 2 + i, 255);
    chip.sine(0, 440);
    chip.regs.write(0x18, 0xC0);  // OP1 = wave C (user wave 2)
    const std::vector<std::int16_t> out = chip.render(2400);
    CHECK(out.back() == out[out.size() - 100]);
    CHECK(out.back() > 3000);
}

// Per-channel taps are the channel's DAC contribution; with one channel they
// equal the master output.
void testTaps() {
    Chip chip;
    chip.sine(9, 440, 0x5C);
    std::vector<std::int16_t> left(512), right(512);
    std::vector<std::int32_t> tapL(512), tapR(512);
    hs816::ChannelOutputs taps;
    taps.left[9] = tapL.data();
    taps.right[9] = tapR.data();
    chip.engine.render(left.data(), right.data(), 512, &taps);
    bool same = true;
    for (int i = 0; i < 512; i++) same = same && tapL[i] == left[i] && tapR[i] == right[i];
    CHECK(same);
}

// Attack 0..1 is linear over A*247.5 samples; decay reaches sustain exactly.
void testEnvelopeShape() {
    hs816::EnvelopeRegisters regs;
    regs.attack = 4;    // 990 samples
    regs.decay = 1;     // 1125 samples
    regs.sustain = 128;
    const hs816::EnvelopeRates rates = hs816::EnvelopeRates::fromRegisters(regs);
    hs816::Envelope env;
    // The level of advance k is k/990; advance 0 reports 0.
    for (int i = 0; i <= 495; i++) env.advance(rates);
    CHECK(std::abs(env.level() - 32768) <= 2);  // halfway through the attack
    for (int i = 0; i < 495; i++) env.advance(rates);
    CHECK(env.stage() == hs816::Envelope::Stage::Decay);
    for (int i = 0; i < 1200; i++) env.advance(rates);
    CHECK(env.stage() == hs816::Envelope::Stage::Sustain);
    CHECK(env.level() == rates.sustain);
    CHECK(hs816::EnvelopeRates::fromRegisters({0, 0, 255, 0}).sustain == 65536);

    env.release();  // zero-length release
    env.advance(hs816::EnvelopeRates::fromRegisters({0, 0, 0, 0}));
    CHECK(env.level() == 0);
}

// The frequency register is exact Hz: no quantization to a 192 kHz period.
// 11000 Hz would have played as 192000 / floor(192000 / 11000) = 11294 Hz.
void testFrequencyIsExact() {
    for (int hz : {11000, 4401, 37}) {
        Chip chip;
        chip.sine(0, hz);
        const int cycles = risingCrossings(chip.render(hs816::kSampleRate));
        CHECK(std::abs(cycles - hz) <= 1);
    }
}

}  // namespace

int main() {
    testIdAndAddressSpace();
    testDecoding();
    testSilentAtPowerOn();
    testGate();
    testRetriggerResetsPhase();
    testLongNoteStaysPeriodic();
    testPanMuteAndChannelEquivalence();
    testUserWave();
    testTaps();
    testEnvelopeShape();
    testFrequencyIsExact();
    if (failures == 0) std::printf("all 3HS816 tests passed\n");
    return failures == 0 ? 0 : 1;
}
