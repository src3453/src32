#include <array>
#include <cstdint>
#include <memory>
#include <vector>

/*
3WS8PN (S3W2) Specification
=========================
Overall: Fantasy wavetable soundchip with 8 of wavetable/PCM/noise channels
Wavetable: Size: 256x256
Wavetable RAM: 16kbit SRAM
Wavetable Modulation: with combining two wavetables (ex. CH1+CH2, CH3+CH8)... Phase Modulation, Ring Modulation, Hard Sync, Window, ...
PCM: 8bit, RAM Max 8Mbit (1MB), 20bit I/O (also can output 16bit with DMA, no volume control)
Noise: 32-bit LFSR, 1bit output, customizable LFSR Tap
Mixer: 16bit Stereo Linear PCM, 48KHz Master Output, each channel has 256 volume steps, 16 panpot (each stereo channel has 16 steps)
Master Clock: 9.216MHz, Main Sample Clock, divived by 48, 192KHz, all sound frequencies were quantized by this value
Registers (all bytes are in big endian):
0x000-0x7FF: Channel Wavetable RAM (8 channels x 256 bytes)
 - +0x00~0xFF: Wavetable Data (256 bytes)
0x800-0x8FF: Channel Control Registers (8 channels x 32 bytes)
 - +0x00~0x01: Frequency (in Hertz, 16bit, in PCM Mode: Sample Rate (32x Frequency (ex. 64hz -> 0x0002)))
 - +0x02: Waveform Type (0: Wavetable, 1: PCM, 2: Noise, 3: DMA PCM (16bit), 4~: Reserved)
 - +0x03: Volume (0~255)
 - +0x04: Panpot (MSB: Left 4bit, LSB: Right 4bit)
 - +0x05: Waveform Modulation Type (MSB 5bit: Type, LSB 3bit: Target Channel (0~7))
 - +0x06~0x09: Modulation Depth/Parameter (depends on Modulation Type)
 - +0x0A: Any access in this register will reset the channel phase
 - +0x0B: Modulation Targeting Mode (bit0: 0=Absolute, 1=Relative, bit1~7: Reserved)
  in PCM Mode:
 - +0x10~0x12: PCM Start Address (MSB 4bit: Reserved, LSB 20bit: Address)
 - +0x13~0x15: PCM End Address (MSB 4bit: Reserved, LSB 20bit: Address)
 - +0x16~0x18: PCM Loop Address (MSB 4bit: Reserved, LSB 20bit: Address)
 - +0x19: PCM Playing Control (bit0: Play/Stop, bit1: Loop On/Off, bit2~7: Reserved)
*/

// 定数定義
constexpr int NUM_CHANNELS = 8;
constexpr int WAVETABLE_SIZE = 256;
constexpr int PCM_RAM_SIZE = 1024 * 1024;  // 1MB
constexpr int SAMPLE_RATE = 48000;         // 48KHz
constexpr int MASTER_CLOCK = 9216000;      // 9.216MHz
constexpr int SOUND_CLOCK = 192000;        // 192KHz (9.216MHz / 48)

// 波形タイプ
enum WaveformType {
    WAVEFORM_WAVETABLE = 0,
    WAVEFORM_PCM = 1,
    WAVEFORM_NOISE = 2,
    WAVEFORM_DMA_PCM = 3
};

// 変調タイプ
enum ModulationType {
    MOD_NONE = 0,
    MOD_PHASE = 1,
    MOD_RING = 2,
    MOD_HARD_SYNC = 3,
    MOD_WINDOW = 4
};

// チャンネル制御構造体
struct Channel {
    // レジスタデータ
    uint16_t frequency;          // 周波数 (Hz)
    uint8_t waveform_type;       // 波形タイプ
    uint8_t volume;              // ボリューム (0-255)
    uint8_t panpot;              // パンポット (上位4bit:L, 下位4bit:R)
    uint8_t modulation_type;     // 変調タイプ
    uint8_t modulation_target;   // 変調ターゲットチャンネル (0~7, 3bit)
    uint8_t modulation_targeting_mode; // 変調ターゲットチャンネルの選択モード (0: 絶対アドレス、 1: 相対アドレス (3bit signed, -4~+3))
    uint16_t modulation_param_1;   // 変調パラメータ1
    uint16_t modulation_param_2;   // 変調パラメータ2
    
    // PCM用レジスタ
    uint32_t pcm_start_addr;     // PCM開始アドレス (20bit)
    uint32_t pcm_end_addr;       // PCM終了アドレス (20bit)
    uint32_t pcm_loop_addr;      // PCMループアドレス (20bit)
    uint8_t pcm_control;         // PCM制御レジスタ
    
    // 内部状態
    uint64_t old_phase;
    uint32_t lfsr_state;         // ノイズ用LFSR状態
    double phase;              // 現在の位相 (1 = 1サンプル (in 192KHz) , 256 = 1周期)
    int16_t last_sample;        // 最後に出力したサンプル値
    bool active;                 // チャンネルが有効かどうか
    
    // 波形テーブルデータ
    std::array<uint8_t, WAVETABLE_SIZE> wavetable;
    
    Channel() : frequency(0), waveform_type(0), volume(0), panpot(0xFF),
                modulation_type(0), modulation_param_1(0), modulation_param_2(0),
                pcm_start_addr(0), pcm_end_addr(0), pcm_loop_addr(0), pcm_control(0),
                phase(0.0), lfsr_state(1), last_sample(0), active(false) {
        wavetable.fill(0x80);  // 中央値で初期化
    }
};

class S3W2_Sound {
public:
    S3W2_Sound();
    ~S3W2_Sound();
    void initialize();
    void reset();
    void writeRegister(uint32_t address, uint8_t value);
    uint8_t readRegister(uint32_t address);
    // PCMRAM
    void writePCMRAM(uint32_t address, uint8_t value);
    uint8_t readPCMRAM(uint32_t address);
    std::vector<std::vector<std::vector<int16_t>>> clock(size_t samples); // final output
    int16_t generateSample(int ch); // routing function

private:
    // チャンネルデータ
    std::array<Channel, NUM_CHANNELS> channels;
    
    // PCM RAM (1MB)
    std::unique_ptr<std::array<uint8_t, PCM_RAM_SIZE>> pcm_ram;

    // 内部メソッド
    void resetChannel(int ch);
    int16_t generateWavetableSample(int ch);
    int16_t generatePCMSample(int ch);
    int16_t generateNoiseSample(int ch);
    int16_t generateDMAPCMSample(int ch);
    int16_t applyModulation(int ch, int16_t& sample);
    void mixChannels(int16_t* left, int16_t* right);
    
    // レジスタ操作ヘルパー
    void writeChannelWavetable(int ch, uint8_t offset, uint8_t value);
    void writeChannelPCMReg(int ch, uint8_t offset, uint8_t value);
    void writeChannelControl(int ch, uint8_t offset, uint8_t value);
    uint8_t readChannelWavetable(int ch, uint8_t offset);
    uint8_t readChannelPCMReg(int ch, uint8_t offset);
    uint8_t readChannelControl(int ch, uint8_t offset);
    
    // ユーティリティ
    uint8_t convertToAbsoluteChannelAddress(uint8_t carrier_channel, uint8_t modulation_targeting_mode, uint8_t modulation_target);
    uint32_t extractAddress20bit(uint32_t reg_addr);
    void setAddress20bit(uint32_t& target, uint32_t reg_addr, uint8_t value);
};