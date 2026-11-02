// Comparison of the integer 3HS816 engine against the float 3HS88PWN4
// reference (Furnace src/engine/platform/sound/s3hs88pwn4/sound.cpp, which
// this test only reads).
//
// The built-in waves must match the reference exactly. Audio cannot be
// bit-exact (Q32 phase, integer envelope and modulation instead of float), so
// each scenario drives the reference's 8 harmonic channels and the same
// settings on 3HS816 channels 0..7 and again on 8..15, and requires the
// master output to stay above a signal-to-error ratio. The reference's
// PCM/WT channels stay at volume 0 and only supply waves A..D; its EQ and
// compressor stay off (3HS816 has none).
#include HS816_LEGACY_SOURCE

#include <cmath>
#include <cstdio>
#include <functional>
#include <memory>
#include <string>
#include <vector>

#include "hs816/registers.hpp"

namespace {

constexpr int kBlock = 256;

class Pair {
public:
    explicit Pair(int channelOffset) : offset_(channelOffset), regs_(engine_) {
        legacy_ = std::make_unique<S3HS_sound>();
        legacy_->initSound();
        // The reference's 0 Hz path converts infinity to int (undefined);
        // give its PCM/WT channels a defined pitch. They stay at volume 0.
        for (int w = 0; w < 4; w++) {
            poke(0x200 + 48 * w, 0x02);
            poke(0x201 + 48 * w, 0xEE);
        }
    }

    void channel(int ch, int offset, int value) {
        poke(64 * ch + offset, value);
        regs_.write(static_cast<std::uint32_t>(64 * (ch + offset_) + offset), static_cast<std::uint8_t>(value));
    }
    void frequency(int ch, int hz) {
        channel(ch, 0, hz >> 8);
        channel(ch, 1, hz & 0xFF);
    }
    void ratio(int ch, int op, int q12) {
        channel(ch, op * 2, q12 >> 8);
        channel(ch, op * 2 + 1, q12 & 0xFF);
    }
    void keyOn(int ch) {
        // Furnace's note-on: resetGate() then gate=1.
        legacy_->resetGate(ch);
        channel(ch, 0x1E, 1);
    }
    void keyOff(int ch) { channel(ch, 0x1E, 0); }
    void feedbackTarget(int ch, int op) {
        poke(0x3C0 + ch, op);
        regs_.write(static_cast<std::uint32_t>(0x490 + ch + offset_), static_cast<std::uint8_t>(op));
    }
    void mute(int ch, int value) {
        poke(0x2D0 + ch, value);
        regs_.write(static_cast<std::uint32_t>(0x480 + ch + offset_), static_cast<std::uint8_t>(value));
    }
    void userWave(int wave, int index, int value) {
        poke(0x210 + 48 * wave + index, value);
        regs_.write(static_cast<std::uint32_t>(0x400 + 32 * wave + index), static_cast<std::uint8_t>(value));
    }

    // Render one block from both and accumulate signal and error energy.
    void renderBlock() {
        auto frames = legacy_->AudioCallBack(kBlock);
        std::vector<std::int16_t> left(kBlock), right(kBlock);
        engine_.render(left.data(), right.data(), kBlock);
        for (int i = 0; i < kBlock; i++) {
            const int ref[2] = {static_cast<std::int16_t>(frames[0][12][i]), static_cast<std::int16_t>(frames[1][12][i])};
            const int got[2] = {left[i], right[i]};
            for (int side = 0; side < 2; side++) {
                const double error = got[side] - ref[side];
                signal_ += static_cast<double>(ref[side]) * ref[side];
                error_ += error * error;
                maxError_ = std::max(maxError_, std::abs(got[side] - ref[side]));
            }
        }
    }

    // Signal-to-error ratio in dB; +inf when identical.
    double snr() const { return error_ == 0 ? INFINITY : 10 * std::log10(signal_ / error_); }
    int maxError() const { return maxError_; }
    bool silent() const { return signal_ == 0; }

    bool wavesMatch() const {
        for (int wave = 0; wave < 16; wave++) {
            if (wave >= 10 && wave <= 13) continue;  // user waves
            for (int i = 0; i < 256; i++) {
                if (legacy_->sintable[wave][i] != engine_.waves().sample(wave, i)) {
                    std::printf("FAIL wave %X[%d]: ref %d got %d\n", wave, i, legacy_->sintable[wave][i],
                                engine_.waves().sample(wave, i));
                    return false;
                }
            }
        }
        return true;
    }

private:
    void poke(int offset, int value) {
        legacy_->ram_poke(legacy_->ram, 0x400000 + offset, static_cast<Byte>(value));
    }

    int offset_;
    hs816::Engine engine_;
    hs816::RegisterInterface regs_;
    std::unique_ptr<S3HS_sound> legacy_;
    double signal_ = 0;
    double error_ = 0;
    int maxError_ = 0;
};

// How a scenario is judged.
//  Exact: every quantity is exactly representable in the float reference
//         (750/375 Hz, integer ratios, flat envelopes), so outputs must agree
//         to 1 LSB. This pins routing, modulation, feedback, the lookup dead
//         zone and the DAC scale for all algorithms.
//  Close: realistic pitches and envelopes without FM; the error is the
//         reference's float rounding plus 1-LSB envelope steps.
//  Info:  realistic FM. The reference rounds every lookup phase to float32
//         (about 1e-3 table steps of noise), which decides boundary hits that
//         FM then amplifies, so it is reported but not checked.
enum class Check { Exact, Close, Info };

struct Scenario {
    std::string name;
    Check check;
    int blocks;
    std::function<void(Pair&)> setup;
    // Called before rendering block b (b >= 1).
    std::function<void(Pair&, int)> between = nullptr;
};

constexpr double kCloseMinSnr = 40.0;

struct Envelope {
    int attack, decay, sustain, release;
};
constexpr Envelope kFlat = {0, 0, 255, 0};

// A full voice on channel `ch`. `exact` keeps every ratio integral and the
// envelope flat so that the reference computes without rounding.
void voice(Pair& p, int ch, int hz, int algorithm, int waveSeed, int feedback, bool exact) {
    p.frequency(ch, hz);
    static const int realRatios[7] = {0x2000, 0x3000, 0x1800, 0x0800, 0x2800, 0x1000, 0x3400};
    static const int exactRatios[7] = {0x2000, 0x3000, 0x1000, 0x4000, 0x2000, 0x1000, 0x3000};
    for (int op = 1; op < 8; op++) {
        p.ratio(ch, op, (exact ? exactRatios : realRatios)[(op + ch) % 7]);
    }
    for (int op = 0; op < 8; op++) {
        const Envelope env = exact ? kFlat : Envelope{op % 3, 3 + op, 120 + 15 * op, 4 + op};
        p.channel(ch, 0x10 + op, 90 + 19 * op);
        p.channel(ch, 0x20 + op * 4, env.attack);
        p.channel(ch, 0x21 + op * 4, env.decay);
        p.channel(ch, 0x22 + op * 4, env.sustain);
        p.channel(ch, 0x23 + op * 4, env.release);
    }
    for (int i = 0; i < 4; i++) {
        const int a = (waveSeed + 2 * i) % 16;
        const int b = (waveSeed + 2 * i + 1) % 16;
        p.channel(ch, 0x18 + i, a << 4 | b);
    }
    p.channel(ch, 0x1C, algorithm);
    p.channel(ch, 0x1F, feedback);
    p.keyOn(ch);
}

void fillUserWaves(Pair& p) {
    for (int w = 0; w < 4; w++) {
        for (int i = 0; i < 32; i++) {
            p.userWave(w, i, (i * 7 + w * 31 + (i * i) % 13) % 256);
        }
    }
}

bool isFm(int algorithm) {
    return algorithm != 0 && algorithm != 2 && algorithm != 8;
}

// The reference quantizes F to 192000 / floor(192000 / F); 3HS816/3HS88PWN4
// play F exactly. Every frequency here divides 192000, where both agree.
std::vector<Scenario> scenarios() {
    std::vector<Scenario> list;
    const std::string alg = "algorithm ";

    for (int a = 0; a < 13; a++) {
        list.push_back({"exact: " + alg + std::to_string(a), Check::Exact, 8, [a](Pair& p) {
                            fillUserWaves(p);
                            voice(p, 0, 750, a, a, 128, true);
                            voice(p, 5, 375, (a + 4) % 13, a + 5, 128, true);
                            p.channel(5, 0x1D, 0x3E);
                        }});
    }
    for (int wave = 0; wave < 16; wave++) {
        list.push_back({"exact: wave " + std::to_string(wave), Check::Exact, 4, [wave](Pair& p) {
                            fillUserWaves(p);
                            voice(p, 2, 750, 0, 0, 128, true);
                            for (int i = 0; i < 4; i++) p.channel(2, 0x18 + i, wave << 4 | wave);
                            voice(p, 3, 375, 4, 0, 128, true);
                            for (int i = 0; i < 4; i++) p.channel(3, 0x18 + i, wave << 4 | wave);
                        }});
    }
    list.push_back({"exact: pan, mute, saturation", Check::Exact, 6,
                    [](Pair& p) {
                        for (int ch = 0; ch < 8; ch++) {
                            voice(p, ch, ch % 2 ? 375 : 750, ch % 13, ch, 128, true);
                            for (int op = 0; op < 8; op++) p.channel(ch, 0x10 + op, 255);
                            p.channel(ch, 0x1D, (ch * 37) & 0xFF);
                        }
                        p.mute(2, 1);
                        p.mute(5, 7);
                    },
                    [](Pair& p, int block) {
                        if (block == 3) {
                            p.mute(2, 0);
                            p.mute(6, 1);
                        }
                    }});
    list.push_back({"exact: feedback target out of range, algorithm >= 13", Check::Exact, 4, [](Pair& p) {
                        voice(p, 0, 750, 4, 0, 250, true);
                        p.feedbackTarget(0, 8);
                        voice(p, 1, 375, 6, 2, 10, true);
                        p.feedbackTarget(1, 255);
                        voice(p, 2, 750, 13, 0, 128, true);
                        voice(p, 3, 750, 255, 0, 128, true);
                    }});

    for (int a : {0, 2, 8}) {
        list.push_back({"close: " + alg + std::to_string(a) + " with envelopes", Check::Close, 8, [a](Pair& p) {
                            fillUserWaves(p);
                            voice(p, 1, 480, a, a, 128, false);
                            voice(p, 6, 250, a, a + 3, 128, false);
                            p.channel(6, 0x1D, 0x4F);
                        }});
    }
    list.push_back({"close: release, re-release, retrigger, legato gate", Check::Close, 12,
                    [](Pair& p) {
                        voice(p, 0, 250, 0, 0, 128, false);
                        voice(p, 6, 200, 2, 3, 128, false);
                    },
                    [](Pair& p, int block) {
                        if (block == 3) p.keyOff(0);
                        if (block == 4) p.keyOff(0);  // already off: no effect
                        if (block == 5) p.keyOff(6);
                        if (block == 7) p.keyOn(0);   // retrigger after release
                        if (block == 9) p.keyOn(6);
                        if (block == 10) p.keyOn(6);  // retrigger while gate on
                        if (block == 10) p.channel(0, 0x1E, 2);  // stored only
                    }});
    list.push_back({"close: pitch, ratio and level changes between blocks", Check::Close, 8,
                    [](Pair& p) { voice(p, 4, 100, 0, 1, 128, false); },
                    [](Pair& p, int block) {
                        static const int pitches[8] = {100, 128, 150, 160, 192, 200, 240, 250};
                        p.frequency(4, pitches[block]);
                        p.ratio(4, 3, 0x1234 + block * 0x111);
                        p.channel(4, 0x1C, block % 2 ? 8 : 0);
                        p.channel(4, 0x10 + block % 8, 255 - block * 20);
                        p.channel(4, 0x21 + (block % 8) * 4, block);
                    }});

    for (int a = 0; a < 13; a++) {
        if (!isFm(a)) continue;
        list.push_back({"info: " + alg + std::to_string(a) + " FM with envelopes, feedback", Check::Info, 8,
                        [a](Pair& p) {
                            fillUserWaves(p);
                            voice(p, 1, 320, a, a, 200, false);
                            p.feedbackTarget(1, a % 8);
                        }});
    }
    list.push_back({"info: power-on defaults (feedback -1 on OP1, no key-on)", Check::Info, 4, [](Pair& p) {
                        p.frequency(0, 480);
                        for (int op = 0; op < 8; op++) {
                            p.channel(0, 0x10 + op, 200);
                            p.channel(0, 0x22 + op * 4, 255);
                        }
                    }});

    return list;
}

}  // namespace

int main() {
    int failures = 0;
    int checks = 0;
    {
        Pair pair(0);
        if (!pair.wavesMatch()) failures++;
        checks++;
    }
    std::printf("%-58s %6s %9s %8s\n", "scenario", "offset", "SNR(dB)", "maxerr");
    for (const Scenario& scenario : scenarios()) {
        for (int offset : {0, 8}) {
            Pair pair(offset);
            scenario.setup(pair);
            pair.renderBlock();
            for (int block = 1; block < scenario.blocks; block++) {
                if (scenario.between) scenario.between(pair, block);
                pair.renderBlock();
            }
            bool ok = true;
            if (scenario.check == Check::Exact) ok = pair.maxError() <= 1;
            if (scenario.check == Check::Close) ok = pair.snr() >= kCloseMinSnr;
            std::printf("%-58s %6d %9.1f %8d %s\n", scenario.name.c_str(), offset, pair.snr(), pair.maxError(),
                        ok ? "" : "FAIL");
            if (scenario.check != Check::Info) {
                checks++;
                if (!ok) failures++;
            }
        }
    }
    std::printf("%d/%d checked legacy comparisons passed\n", checks - failures, checks);
    return failures == 0 ? 0 : 1;
}
