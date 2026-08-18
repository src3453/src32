#include "sound.hpp"
#include <cstring>
#include <algorithm>
#include <cmath>
#include <stdexcept>

#define CLIP(x,min,max) ((x)<(min)?(min):((x)>(max)?(max):(x)))

S3W2_Sound::S3W2_Sound()
{
    initialize();
}

S3W2_Sound::~S3W2_Sound()
{
    pcm_ram.reset();
}

void S3W2_Sound::initialize()
{
    // allocate PCM RAM
    pcm_ram = std::make_unique<std::array<uint8_t, PCM_RAM_SIZE>>();
    std::fill(pcm_ram->begin(), pcm_ram->end(), 0);
    for (int i = 0; i < NUM_CHANNELS; ++i)
        resetChannel(i);
}

void S3W2_Sound::reset()
{
    if (pcm_ram)
        std::fill(pcm_ram->begin(), pcm_ram->end(), 0);
    for (int i = 0; i < NUM_CHANNELS; ++i)
        resetChannel(i);
}

void S3W2_Sound::resetChannel(int ch)
{
    if (ch < 0 || ch >= NUM_CHANNELS)
        return;
    Channel &c = channels[ch];
    c.frequency = 0;
    c.waveform_type = WAVEFORM_WAVETABLE;
    c.volume = 0;
    c.panpot = 0xFF;
    c.modulation_type = MOD_NONE;
    c.modulation_param_1 = 0;
    c.modulation_param_2 = 0;
    c.modulation_target = 0;
    c.modulation_targeting_mode = 0;
    c.pcm_start_addr = 0;
    c.pcm_end_addr = 0;
    c.pcm_loop_addr = 0;
    c.pcm_control = 0;
    c.phase = 0.0;
    c.last_sample = 0;
    c.lfsr_state = 0x12D4803C;
    c.active = true;
    c.wavetable.fill(0x80);
}

void S3W2_Sound::writeRegister(uint32_t address, uint8_t value)
{
    // basic routing: wavetable area 0x000 - 0x7FF, control area 0x800 - 0x8FF
    if (address <= 0x7FF)
    {
        // each channel has 256 bytes, 8 channels => 0x000-0x7FF
        uint32_t ch = (address >> 8) & 0x7; // address / 256
        uint8_t offset = address & 0xFF;
        writeChannelWavetable(static_cast<int>(ch), offset, value);
        return;
    }
    else if (address >= 0x800 && address <= 0x8FF)
    {
        uint32_t ch = ((address - 0x800) >> 5) & 0x7; // each channel 32 bytes
        uint8_t offset = (address - 0x800) & 0x1F;
        if (offset < 0x10) {
            writeChannelControl(static_cast<int>(ch), offset, value);
        } else {
            writeChannelPCMReg(static_cast<int>(ch), offset - 0x10, value);
        }
        return;
    }
    // other addresses not implemented yet
    (void)address;
    (void)value;
}

uint8_t S3W2_Sound::readRegister(uint32_t address)
{
    if (address <= 0x7FF)
    {
        uint32_t ch = (address >> 8) & 0x7;
        uint8_t offset = address & 0xFF;
        return readChannelWavetable(static_cast<int>(ch), offset);
    }
    else if (address >= 0x800 && address <= 0x8FF)
    {
        uint32_t ch = ((address - 0x800) >> 5) & 0x7;
        uint8_t offset = (address - 0x800) & 0x1F;
        return readChannelControl(static_cast<int>(ch), offset);
    }
    return 0;
}

std::vector<std::vector<std::vector<int16_t>>> S3W2_Sound::clock(size_t samples)
{
  // Prepare output buffer: [channel][left/right][sample]
    std::vector<std::vector<std::vector<int16_t>>> output(NUM_CHANNELS,
        std::vector<std::vector<int16_t>>(2, std::vector<int16_t>(samples, 0)));
    for (size_t i = 0; i < samples; ++i)
    {
        for (int ch = 0; ch < NUM_CHANNELS; ++ch)
        {
            for (int j = 0; j < 4; ++j)
            {
                int16_t sample = generateSample(ch);
                uint8_t panL = channels[ch].panpot >> 4; // upper 4 bits for left
                uint8_t panR = channels[ch].panpot & 0x0F; // lower 4 bits for right
                output[ch][0][i] += sample * panL / 15; // oversampling x4 192KHz -> 48KHz
                output[ch][1][i] += sample * panR / 15; // oversampling x4 192KHz -> 48KHz
                //printf("Ch %d Sample %d, Subsample %d: %d\n", ch, i, j, sample);
            }
        }
    }
    return output;
}

int16_t S3W2_Sound::generateSample(int ch)
{
    if (ch < 0 || ch >= NUM_CHANNELS)
        return 0;
    Channel &c = channels[ch];
    if (!c.active)
        return 0;
    int16_t sample = 0;
    switch (c.waveform_type)
    {
    case WAVEFORM_WAVETABLE:
        sample = generateWavetableSample(ch);
        break;
    case WAVEFORM_PCM:
        sample = generatePCMSample(ch);
        break;
    case WAVEFORM_NOISE:
        sample = generateNoiseSample(ch);
        break;
    case WAVEFORM_DMA_PCM:
        sample = generateDMAPCMSample(ch);
        break;
    default:
        sample = 0;
        break;
    }
    c.last_sample = sample;
    return sample * c.volume / 4;
}

int16_t S3W2_Sound::generateWavetableSample(int ch)
{
    if (ch < 0 || ch >= NUM_CHANNELS)
        return 0;
    Channel &c = channels[ch];
    if (c.waveform_type != WAVEFORM_WAVETABLE)
        return 0;
    // calculate phase increment
    c.phase+=static_cast<double>(c.frequency)/SOUND_CLOCK; // advance phase
    uint8_t index = 0;
    int16_t phase_offset = 0;
    uint64_t phase = static_cast<uint64_t>(c.phase*256) & 0xFFFFFFFFFFFFFFFF;
    uint8_t abs_target_ch = convertToAbsoluteChannelAddress(static_cast<uint8_t>(ch), c.modulation_targeting_mode, c.modulation_target);
    switch (c.modulation_type)
    {
    case MOD_NONE:
        index = static_cast<uint8_t>(phase & 0xFF);
        break;
    case MOD_PHASE:
        phase_offset = static_cast<int16_t>((c.modulation_param_1 * channels[abs_target_ch].last_sample) >> 12);
        index = static_cast<uint8_t>((phase + phase_offset) & 0xFF);
        break;
    default:
        index = static_cast<uint8_t>(phase & 0xFF);
        break;
    }
    uint8_t sample8 = c.wavetable[index];
    if (c.modulation_type == MOD_RING)
    {
        int16_t mod_sample = channels[abs_target_ch].last_sample;
        uint8_t org_sample8 = sample8;
        sample8 = static_cast<uint8_t>CLIP((((static_cast<int>(org_sample8) - 128) * (mod_sample)) * (c.modulation_param_1)) / 65536 + 128, 0, 255);
        //printf("Ring Mod Ch %d Target Ch %d Org %d Sample %d ModSample %d\n", ch, abs_target_ch, org_sample8, sample8, mod_sample);
    }
    else if (c.modulation_type == MOD_HARD_SYNC)
    {
        if (channels[abs_target_ch].phase < 1.0)
        {
            c.phase = 0.0; // reset phase
            index = 0;
            sample8 = c.wavetable[index];
        }
    }
    else if (c.modulation_type == MOD_WINDOW)
    {
        int16_t mod_sample = channels[abs_target_ch].last_sample;
        if (mod_sample < 0)
            sample8 = 128; // silence
    }
    int16_t out = static_cast<int16_t>((static_cast<int>(sample8) - 128));
    return out;
}

int16_t S3W2_Sound::generatePCMSample(int ch)
{
    if (ch < 0 || ch >= NUM_CHANNELS)
        return 0;
    Channel &c = channels[ch];
    if (c.waveform_type != WAVEFORM_PCM)
        return 0;
    // bounds check
    if (!pcm_ram)
        return 0;
    uint64_t phase = static_cast<uint64_t>(c.phase*256) & 0xFFFFFFFFFFFFFFFF;
    uint8_t abs_target_ch = convertToAbsoluteChannelAddress(static_cast<uint8_t>(ch), c.modulation_targeting_mode, c.modulation_target);
    if (c.modulation_type == MOD_PHASE)
    {
        int16_t phase_offset = static_cast<int16_t>((c.modulation_param_1 * channels[abs_target_ch].last_sample) >> 12);
        phase = (phase + phase_offset) & 0xFFFFFFFFFFFFFFFF;
    }
    uint32_t addr = (phase+c.pcm_start_addr) & (PCM_RAM_SIZE - 1);
    uint8_t sample8 = (*pcm_ram)[addr];
    int16_t out = static_cast<int16_t>((static_cast<int>(sample8) - 128));
    // advance
    if (addr >= c.pcm_end_addr)
    {
        if (c.pcm_control & 0x02)
        { // loop on 
            c.phase = (c.pcm_loop_addr-c.pcm_start_addr)/256.0;
        }
        else
        {
            c.phase = (c.pcm_end_addr-c.pcm_start_addr)/256.0;
            out = 0; // silence after end
        }
    } else {
        c.phase+=static_cast<double>(c.frequency)/SOUND_CLOCK/8; // advance phase
    }
    //printf("PCM Ch %d curAddr %05X startAddr %05X endAddr %05X loopAddr %05X Sample %d\n", ch, addr, c.pcm_start_addr, c.pcm_end_addr, c.pcm_loop_addr, out);
    return out;
}

#define MOD_NOISE_CUSTOM_TAP MOD_PHASE

int16_t S3W2_Sound::generateNoiseSample(int ch)
{
    if (ch < 0 || ch >= NUM_CHANNELS)
        return 0;
    Channel &c = channels[ch];
    if (c.waveform_type != WAVEFORM_NOISE)
        return 0;
    // simple 1-bit LFSR -> -32768 or +32767
    // advance
    c.phase+=static_cast<double>(c.frequency)/SOUND_CLOCK; // advance phase
    uint64_t phase = static_cast<uint64_t>(c.phase*256) & 0xFFFFFFFFFFFFFFFF;
    if (c.old_phase/8 != phase/8) {
        c.old_phase = static_cast<uint64_t>(c.phase*256) & 0xFFFFFFFFFFFFFFFF;
        uint32_t bit = 0;
        uint32_t taps = 0;
        switch (c.modulation_type)
        {
            case MOD_NONE:
                bit = ((c.lfsr_state >> 0) ^ (c.lfsr_state >> 1)) & 1;
                c.lfsr_state = (c.lfsr_state >> 1) | (bit << 22); // update LFSR
                break;
            case MOD_NOISE_CUSTOM_TAP:
                taps = c.modulation_param_1 << 16 | c.modulation_param_2; // as 23-bit tap mask (ignore upper bits)
                bit = 0;
                for (int i = 0; i < 23; ++i)
                {
                    if (taps & (1 << i))
                        bit ^= (c.lfsr_state >> i) & 1;
                }
                c.lfsr_state = (c.lfsr_state >> 1) | (bit << 22); // update LFSR
                break;
            default:
                bit = ((c.lfsr_state >> 0) ^ (c.lfsr_state >> 1)) & 1;
                c.lfsr_state = (c.lfsr_state >> 1) | (bit << 22); // update LFSR
                break;
        }
    }
    int16_t out = (c.lfsr_state & 1) ? 127 : -128;
    return out;
}

int16_t S3W2_Sound::generateDMAPCMSample(int ch)
{
    // not implemented yet, raise error
    throw std::runtime_error("DMA PCM not implemented");
}

int16_t S3W2_Sound::applyModulation(int ch, int16_t &sample)
{
    // placeholder: no modulation applied yet
    return static_cast<int16_t>(sample);
}

void S3W2_Sound::mixChannels(int16_t *left, int16_t *right)
{
    // placeholder
    (void)left;
    (void)right;
}

void S3W2_Sound::writeChannelWavetable(int ch, uint8_t offset, uint8_t value)
{
    if (ch < 0 || ch >= NUM_CHANNELS)
        return;
    if (offset >= WAVETABLE_SIZE)
        return;
    channels[ch].wavetable[offset] = value;
}

void S3W2_Sound::writeChannelPCMReg(int ch, uint8_t offset, uint8_t value)
{
    if (ch < 0 || ch >= NUM_CHANNELS)
        return;
    Channel &c = channels[ch];
    // offset mapping: 0-2 start addr, 3-5 end, 6-8 loop, 9 control
    if (offset <= 2)
    {
        // start addr is 20-bit across 3 bytes big-endian. We'll write per-byte
        uint32_t shift = (2 - offset) * 8;
        uint32_t mask = 0xFFu << shift;
        uint32_t v = (static_cast<uint32_t>(value) << shift);
        c.pcm_start_addr = (c.pcm_start_addr & ~mask) | v;
    }
    else if (offset >= 3 && offset <= 5)
    {
        uint32_t shift = (5 - offset) * 8;
        uint32_t mask = 0xFFu << shift;
        uint32_t v = (static_cast<uint32_t>(value) << shift);
        c.pcm_end_addr = (c.pcm_end_addr & ~mask) | v;
    }
    else if (offset >= 6 && offset <= 8)
    {
        uint32_t shift = (8 - offset) * 8;
        uint32_t mask = 0xFFu << shift;
        uint32_t v = (static_cast<uint32_t>(value) << shift);
        c.pcm_loop_addr = (c.pcm_loop_addr & ~mask) | v;
    }
    else if (offset == 9)
    {
        c.pcm_control = value;
        if (value & 0x01)
        { // play
            c.active = true;
        }
        else
        {
            c.active = false;
        }
    }
}

void S3W2_Sound::writePCMRAM(uint32_t address, uint8_t value)
{
    if (!pcm_ram)
        return;
    if (address >= PCM_RAM_SIZE)
        return;
    (*pcm_ram)[address] = value;
}

uint8_t S3W2_Sound::readPCMRAM(uint32_t address)
{
    if (!pcm_ram)
        return 0;
    if (address >= PCM_RAM_SIZE)
        return 0;
    return (*pcm_ram)[address];
}

void S3W2_Sound::writeChannelControl(int ch, uint8_t offset, uint8_t value)
{
    if (ch < 0 || ch >= NUM_CHANNELS)
        return;
    Channel &c = channels[ch];
    if (offset == 0)
    {
        // frequency MSB (big endian across 0-1)
        c.frequency = (c.frequency & 0x00FF) | (static_cast<uint16_t>(value) << 8);
    }
    else if (offset == 1)
    {
        c.frequency = (c.frequency & 0xFF00) | static_cast<uint16_t>(value);
    }
    else if (offset == 2)
    {
        c.waveform_type = value;
    }
    else if (offset == 3)
    {
        c.volume = value;
    }
    else if (offset == 4)
    {
        c.panpot = value;
    }
    else if (offset == 5)
    {
        c.modulation_type = static_cast<uint8_t>(value >> 3);
        c.modulation_target = (value & 0x07);
    }
    else if (offset >= 6 && offset <= 9)
    {
        c.modulation_param_1 = (c.modulation_param_1 & ~(0xFFu << ((7 - offset) * 8))) |
                             (static_cast<uint16_t>(value) << ((7 - offset) * 8));
        c.modulation_param_2 = (c.modulation_param_2 & ~(0xFFu << ((9 - offset) * 8))) |
                             (static_cast<uint16_t>(value) << ((9 - offset) * 8));
    }
    else if (offset == 0x0A)
    {
        // access resets phase
        c.phase = 0.0;
        c.lfsr_state = 0x12D4803C;
    }
    else if (offset == 0x0B)
    {
        c.modulation_targeting_mode = value & 0x01;
    }
}

uint8_t S3W2_Sound::readChannelWavetable(int ch, uint8_t offset)
{
    if (ch < 0 || ch >= NUM_CHANNELS)
        return 0;
    if (offset >= WAVETABLE_SIZE)
        return 0;
    return channels[ch].wavetable[offset];
}

uint8_t S3W2_Sound::readChannelPCMReg(int ch, uint8_t offset)
{
    if (ch < 0 || ch >= NUM_CHANNELS)
        return 0;
    Channel &c = channels[ch];
    if (offset <= 2)
    {
        uint32_t shift = (2 - offset) * 8;
        return static_cast<uint8_t>((c.pcm_start_addr >> shift) & 0xFF);
    }
    else if (offset >= 3 && offset <= 5)
    {
        uint32_t shift = (5 - offset) * 8;
        return static_cast<uint8_t>((c.pcm_end_addr >> shift) & 0xFF);
    }
    else if (offset >= 6 && offset <= 8)
    {
        uint32_t shift = (8 - offset) * 8;
        return static_cast<uint8_t>((c.pcm_loop_addr >> shift) & 0xFF);
    }
    else if (offset == 9)
    {
        return c.pcm_control;
    }
    return 0;
}

uint8_t S3W2_Sound::readChannelControl(int ch, uint8_t offset)
{
    if (ch < 0 || ch >= NUM_CHANNELS)
        return 0;
    Channel &c = channels[ch];
    if (offset == 0)
    {
        return static_cast<uint8_t>((c.frequency >> 8) & 0xFF);
    }
    else if (offset == 1)
    {
        return static_cast<uint8_t>(c.frequency & 0xFF);
    }
    else if (offset == 2)
    {
        return c.waveform_type;
    }
    else if (offset == 3)
    {
        return c.volume;
    }
    else if (offset == 4)
    {
        return c.panpot;
    }
    else if (offset == 5)
    {
        return static_cast<uint8_t>((c.modulation_type << 3) | (c.modulation_target & 0x07));
    }
    else if (offset >= 6 && offset <= 9)
    {
        uint32_t shift = (9 - offset) * 8;
        if (offset <= 7)
        {
            return static_cast<uint8_t>((c.modulation_param_1 >> shift) & 0xFF);
        }
        else
        {
            return static_cast<uint8_t>((c.modulation_param_2 >> shift) & 0xFF);
        }
    }
    else if (offset == 0x0A)
    {
        // access resets phase
        c.phase = 0.0;
        return 0;
    }
    else if (offset == 0x0B)
    {
        return c.modulation_targeting_mode & 0x01;
    }
    return 0;
}

uint32_t S3W2_Sound::extractAddress20bit(uint32_t reg_addr)
{
    // reg_addr assumed to point to first of three bytes; big endian
    // Not implemented in this header-only context; kept for API compatibility
    (void)reg_addr;
    return 0;
}

void S3W2_Sound::setAddress20bit(uint32_t &target, uint32_t reg_addr, uint8_t value)
{
    (void)reg_addr;
    (void)value;
    (void)target;
}

// Convert modulation target to absolute channel address
uint8_t S3W2_Sound::convertToAbsoluteChannelAddress(uint8_t carrier_channel, uint8_t modulation_targeting_mode, uint8_t modulation_target)
{
    if (modulation_targeting_mode == 0)
    {
        // absolute addressing
        return modulation_target & 0x07;
    }
    else if (modulation_targeting_mode == 1)
    {
        // relative addressing
        int8_t relative_offset = static_cast<int8_t>(modulation_target & 0x07);
        if (relative_offset >= 4)
            relative_offset -= 8; // convert to signed 3-bit
        int absolute_channel = static_cast<int>(carrier_channel) + relative_offset;
        if (absolute_channel < 0)
            absolute_channel += NUM_CHANNELS;
        else if (absolute_channel >= NUM_CHANNELS)
            absolute_channel -= NUM_CHANNELS;
        absolute_channel &= 0x07; // ensure within 0-7 (wraps around)
        return static_cast<uint8_t>(absolute_channel);
    }
}