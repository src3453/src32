// Comparison of the integer 3HS88PWN4 engine against the float reference
// (Furnace src/engine/platform/sound/s3hs88pwn4/sound.cpp, read-only). Both
// receive the same bus writes, since the register map is the reference's.
//
// Tiers (see doc/spec_3HS816.md):
//  Exact: everything exactly representable in the reference (750/375 Hz,
//         integer ratios, flat envelopes, unfiltered WT/noise); within 1 LSB.
//  Close: realistic non-FM material, filters, EQ and the compressor; SNR.
//  Info:  realistic FM; the reference's float32 lookup phases decide
//         boundary hits that FM amplifies. Reported only.
#include HS816_LEGACY_SOURCE

#include <cmath>
#include <cstdio>
#include <functional>
#include <memory>
#include <string>
#include <vector>

#include "hs88/registers.hpp"

namespace {

constexpr int kBlock = 256;

class Pair {
public:
    Pair() : regs_(engine_) {
        legacy_ = std::make_unique<S3HS_sound>();
        legacy_->initSound();
        // The reference's 0 Hz path converts infinity to int (undefined).
        for (int w = 0; w < 4; w++) frequency(w, 750, true);
    }

    void write(std::uint32_t address, int value) {
        legacy_->ram_poke(legacy_->ram, static_cast<int>(address), static_cast<Byte>(value));
        regs_.write(address, static_cast<std::uint8_t>(value));
    }
    void hs(int ch, int offset, int value) { write(0x400000 + 64 * ch + offset, value); }
    void wt(int ch, int offset, int value) { write(0x400200 + 48 * ch + offset, value); }
    void master(int offset, int value) { write(0x4002C0 + offset, value); }
    void frequency(int ch, int hz, bool pcm) {
        if (pcm) {
            wt(ch, 0, hz >> 8);
            wt(ch, 1, hz & 0xFF);
        } else {
            hs(ch, 0, hz >> 8);
            hs(ch, 1, hz & 0xFF);
        }
    }
    void keyOn(int ch) {
        legacy_->resetGate(ch);  // Furnace's note-on: resetGate() then gate=1
        hs(ch, 0x1E, 1);
    }
    void wtSync(int ch) {
        legacy_->wtSync(ch);
        regs_.write(0x400208 + 48 * ch, 1);
    }

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

    double snr() const { return error_ == 0 ? INFINITY : 10 * std::log10(signal_ / error_); }
    int maxError() const { return maxError_; }

private:
    hs88::Engine engine_;
    hs88::RegisterInterface regs_;
    std::unique_ptr<S3HS_sound> legacy_;
    double signal_ = 0;
    double error_ = 0;
    int maxError_ = 0;
};

enum class Check { Exact, Close, Info };

struct Scenario {
    std::string name;
    Check check;
    int blocks;
    std::function<void(Pair&)> setup;
    std::function<void(Pair&, int)> between = nullptr;
};

constexpr double kCloseMinSnr = 40.0;

void voice(Pair& p, int ch, int hz, int algorithm, int waveSeed, int feedback, bool exact) {
    p.frequency(ch, hz, false);
    static const int realRatios[7] = {0x2000, 0x3000, 0x1800, 0x0800, 0x2800, 0x1000, 0x3400};
    static const int exactRatios[7] = {0x2000, 0x3000, 0x1000, 0x4000, 0x2000, 0x1000, 0x3000};
    for (int op = 1; op < 8; op++) {
        const int r = (exact ? exactRatios : realRatios)[(op + ch) % 7];
        p.hs(ch, op * 2, r >> 8);
        p.hs(ch, op * 2 + 1, r & 0xFF);
    }
    for (int op = 0; op < 8; op++) {
        p.hs(ch, 0x10 + op, 90 + 19 * op);
        p.hs(ch, 0x20 + op * 4, exact ? 0 : op % 3);
        p.hs(ch, 0x21 + op * 4, exact ? 0 : 3 + op);
        p.hs(ch, 0x22 + op * 4, exact ? 255 : 120 + 15 * op);
        p.hs(ch, 0x23 + op * 4, exact ? 0 : 4 + op);
    }
    for (int i = 0; i < 4; i++) {
        p.hs(ch, 0x18 + i, ((waveSeed + 2 * i) % 16) << 4 | (waveSeed + 2 * i + 1) % 16);
    }
    p.hs(ch, 0x1C, algorithm);
    p.hs(ch, 0x1F, feedback);
    p.keyOn(ch);
}

void tables(Pair& p) {
    for (int w = 0; w < 4; w++) {
        for (int i = 0; i < 32; i++) p.wt(w, 0x10 + i, (i * 7 + w * 31 + (i * i) % 13) % 256);
    }
}

// A WT/noise voice on PCM channel `ch`.
void wave(Pair& p, int ch, int hz, int mode, int volume, int pan) {
    p.frequency(ch, hz, true);
    p.wt(ch, 2, volume);
    p.wt(ch, 3, mode);
    p.wt(ch, 7, pan);
    p.wtSync(ch);
}

void pcmSample(Pair& p, int ch, int loopStart) {
    for (int i = 0; i < 4096; i++) p.write(0x1000 + i, static_cast<int>(128 + 100 * std::sin(i * 0.05) + (i % 7)));
    const int start = 0x1000, end = 0x1FFF;
    const int bytes[9] = {start >> 16, start >> 8 & 255, start & 255, end >> 16, end >> 8 & 255, end & 255,
                          loopStart >> 16, loopStart >> 8 & 255, loopStart & 255};
    for (int i = 0; i < 9; i++) p.wt(ch, 0x10 + i, bytes[i]);
}

std::vector<Scenario> scenarios() {
    std::vector<Scenario> list;
    const std::string alg = "algorithm ";
    for (int a = 0; a < 13; a++) {
        list.push_back({"exact: HS " + alg + std::to_string(a), Check::Exact, 8, [a](Pair& p) {
                            tables(p);
                            voice(p, 0, 750, a, a, 128, true);
                            voice(p, 5, 375, (a + 4) % 13, a + 5, 128, true);
                            p.hs(5, 0x1D, 0x3E);
                        }});
    }
    for (int mode : {1, 2, 3, 4}) {
        list.push_back({"exact: WT mode " + std::to_string(mode), Check::Exact, 8, [mode](Pair& p) {
                            tables(p);
                            wave(p, 0, 750, mode, 200, 0xF9);
                            wave(p, 1, 375, mode, 120, 0x4C);
                            wave(p, 2, 750, 7, 30, 0);  // undefined mode: -128 DC
                        }});
    }
    list.push_back({"exact: HS and WT together, mute", Check::Exact, 6,
                    [](Pair& p) {
                        tables(p);
                        voice(p, 3, 750, 1, 2, 128, true);
                        wave(p, 3, 375, 1, 255, 0);
                        p.write(0x4002D0 + 11, 1);
                    },
                    [](Pair& p, int block) {
                        if (block == 3) p.write(0x4002D0 + 11, 0);
                        if (block == 4) p.wtSync(3);
                    }});

    list.push_back({"close: PCM one-shot", Check::Close, 8, [](Pair& p) {
                        pcmSample(p, 0, 0xFFFFFF);
                        wave(p, 0, 440, 0, 220, 0);
                    }});
    list.push_back({"close: PCM looped (reference loop rule)", Check::Close, 12, [](Pair& p) {
                        pcmSample(p, 1, 0x1800);
                        wave(p, 1, 2000, 0, 220, 0x8F);
                    }});
    for (int filter = 0; filter < 5; filter++) {
        list.push_back({"close: WT filter mode " + std::to_string(filter), Check::Close, 8, [filter](Pair& p) {
                            tables(p);
                            wave(p, 2, 220, 1, 255, 0);
                            p.wt(2, 4, filter);
                            p.wt(2, 5, 60);
                            p.wt(2, 6, 90);
                        },
                        [filter](Pair& p, int block) {
                            if (block == 4) p.wt(2, 5, 20 + filter * 30);
                        }});
    }
    list.push_back({"close: noise through band-pass", Check::Close, 8, [](Pair& p) {
                        wave(p, 3, 1000, 2, 255, 0);
                        p.wt(3, 4, 2);
                        p.wt(3, 5, 120);
                        p.wt(3, 6, 20);
                    }});
    list.push_back({"close: additive HS with envelopes", Check::Close, 8, [](Pair& p) {
                        tables(p);
                        voice(p, 1, 440, 0, 0, 128, false);
                        voice(p, 6, 262, 8, 3, 128, false);
                    }});
    const auto mixSetup = [](Pair& p) {
        tables(p);
        voice(p, 0, 440, 0, 0, 128, false);
        voice(p, 2, 330, 2, 4, 128, false);
        wave(p, 0, 220, 1, 255, 0);
        wave(p, 1, 110, 4, 200, 0);
    };
    list.push_back({"close: equalizer", Check::Close, 8,
                    [mixSetup](Pair& p) {
                        mixSetup(p);
                        p.master(1, 1);
                        p.master(5, 24);
                        p.master(6, 200);
                        p.master(7, 40);
                    },
                    [](Pair& p, int block) {
                        if (block == 4) p.master(6, 8);
                    }});
    list.push_back({"close: compressor", Check::Close, 8,
                    [mixSetup](Pair& p) {
                        mixSetup(p);
                        p.master(0, 1);
                        p.master(2, 80);
                        p.master(3, 150);
                        p.master(4, 32);
                    },
                    [](Pair& p, int block) {
                        if (block == 4) p.master(3, 5);
                    }});
    list.push_back({"close: equalizer and compressor", Check::Close, 8, [mixSetup](Pair& p) {
                        mixSetup(p);
                        p.master(0, 1);
                        p.master(1, 1);
                        p.master(2, 120);
                        p.master(3, 255);
                        p.master(4, 48);
                        p.master(5, 16);
                        p.master(6, 12);
                        p.master(7, 30);
                    }});

    for (int a : {1, 4, 6, 11}) {
        list.push_back({"info: HS " + alg + std::to_string(a) + " FM with envelopes", Check::Info, 8, [a](Pair& p) {
                            tables(p);
                            voice(p, 1, 330, a, a, 200, false);
                        }});
    }
    return list;
}

}  // namespace

int main() {
    int failures = 0;
    int checks = 0;
    std::printf("%-50s %9s %8s\n", "scenario", "SNR(dB)", "maxerr");
    for (const Scenario& scenario : scenarios()) {
        Pair pair;
        scenario.setup(pair);
        pair.renderBlock();
        for (int block = 1; block < scenario.blocks; block++) {
            if (scenario.between) scenario.between(pair, block);
            pair.renderBlock();
        }
        bool ok = true;
        if (scenario.check == Check::Exact) ok = pair.maxError() <= 1;
        if (scenario.check == Check::Close) ok = pair.snr() >= kCloseMinSnr;
        std::printf("%-50s %9.1f %8d %s\n", scenario.name.c_str(), pair.snr(), pair.maxError(), ok ? "" : "FAIL");
        if (scenario.check != Check::Info) {
            checks++;
            if (!ok) failures++;
        }
    }
    std::printf("%d/%d checked legacy comparisons passed\n", checks - failures, checks);
    return failures == 0 ? 0 : 1;
}
