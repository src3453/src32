//! Integer-only 3HS816: a line-by-line Rust port of the C++ reference model in
//! `chips/3hs816/` (contract: `doc/spec_3HS816.md`). Every rounding step must
//! stay identical to the C++ model; `tests/reference.rs` checks the output
//! against a fixture rendered by that model.

use crate::tables::{EXP2_NEG, WAVES};

pub const CHANNEL_COUNT: usize = 16;
pub const OPERATOR_COUNT: usize = 8;
pub const SAMPLE_RATE: u32 = 48_000;

const WAVE_COUNT: usize = 16;
const ALGORITHM_COUNT: u8 = 13;
const USER_WAVE_COUNT: usize = 4;
const USER_WAVE_LENGTH: usize = 32;
const FIRST_USER_WAVE: usize = 10;
const RATIO_ONE: u64 = 4096;
const SIGNAL_FRAC_BITS: u32 = 15;
const LEVEL_ONE: i32 = 1 << 16;

// Register map (chip-relative byte addresses).
pub const CHANNEL_STRIDE: u32 = 0x40;
pub const USER_WAVE_BASE: u32 = 0x400;
pub const MUTE_BASE: u32 = 0x480;
pub const FEEDBACK_TARGET_BASE: u32 = 0x490;
pub const ID_BASE: u32 = 0x4F8;
pub const REGISTER_SIZE: u32 = 0x500;
pub const ID: [u8; 8] = [b'3', b'H', b'S', b'8', b'1', b'6', 1, 0];

const REG_RATIO: u32 = 0x02;
const REG_VOLUME: u32 = 0x10;
const REG_WAVE: u32 = 0x18;
const REG_ALGORITHM: u32 = 0x1C;
const REG_PAN: u32 = 0x1D;
const REG_GATE: u32 = 0x1E;
const REG_FEEDBACK: u32 = 0x1F;
const REG_ENVELOPE: u32 = 0x20;

// ---------------------------------------------------------------------------
// Envelope

const ATTACK_UNIT: u32 = 495;
const DECAY_RELEASE_UNIT: u32 = 2250;
const MIN_DECAY: u32 = 1125;
const HALF_SAMPLES_PER_SAMPLE: u32 = 2;
const ELAPSED_LIMIT: u32 = 1 << 20;

#[derive(Clone, Copy, Default)]
struct EnvelopeRegisters {
    attack: u8,
    decay: u8,
    sustain: u8,
    release: u8,
}

#[derive(Clone, Copy, Default)]
struct EnvelopeSegment {
    length: u32,
    reciprocal: u32,
}

impl EnvelopeSegment {
    fn of_length(half_samples: u32) -> Self {
        let reciprocal = if half_samples == 0 {
            0
        } else {
            (((1u64 << 40) + u64::from(half_samples) - 1) / u64::from(half_samples)) as u32
        };
        Self {
            length: half_samples,
            reciprocal,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct EnvelopeRates {
    attack: EnvelopeSegment,
    decay: EnvelopeSegment,
    release: EnvelopeSegment,
    sustain: i32,
}

impl EnvelopeRates {
    fn from_registers(regs: EnvelopeRegisters) -> Self {
        let decay = if regs.decay == 0 {
            MIN_DECAY
        } else {
            DECAY_RELEASE_UNIT * u32::from(regs.decay)
        };
        Self {
            attack: EnvelopeSegment::of_length(ATTACK_UNIT * u32::from(regs.attack)),
            decay: EnvelopeSegment::of_length(decay),
            release: EnvelopeSegment::of_length(DECAY_RELEASE_UNIT * u32::from(regs.release)),
            sustain: i32::from(regs.sustain) * 257 + (i32::from(regs.sustain) >> 7),
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Stage {
    #[default]
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Copy, Default)]
struct Envelope {
    stage: Stage,
    elapsed: u32,
    level: i32,
    release_from: i32,
}

fn position(elapsed: u32, segment: EnvelopeSegment) -> u64 {
    let t = (u64::from(elapsed) * u64::from(segment.reciprocal)) >> 8;
    t.min((1u64 << 32) - 1)
}

fn exponential(from: i32, to: i32, t: u64) -> i32 {
    let y = t * 10;
    let whole = (y >> 32) as u32;
    let fraction = y as u32;
    let index = (fraction >> 24) as usize;
    let weight = (fraction >> 8) & 0xFFFF;
    let hi = EXP2_NEG[index];
    let lo = EXP2_NEG[index + 1];
    let power = hi - (((hi - lo) * weight) >> 16);
    let shape = i64::from(LEVEL_ONE) - i64::from(power >> whole);
    from + ((i64::from(to - from) * shape) >> 16) as i32
}

impl Envelope {
    fn restart(&mut self) {
        self.elapsed = 0;
        self.stage = Stage::Attack;
    }

    fn release(&mut self) {
        if self.stage != Stage::Release {
            self.elapsed = 0;
            self.stage = Stage::Release;
        }
        self.release_from = self.level;
    }

    fn advance(&mut self, rates: &EnvelopeRates) {
        loop {
            match self.stage {
                Stage::Attack => {
                    if self.elapsed < rates.attack.length {
                        self.level = (position(self.elapsed, rates.attack) >> 16) as i32;
                        break;
                    }
                    self.elapsed -= rates.attack.length;
                    self.stage = Stage::Decay;
                }
                Stage::Decay => {
                    if self.elapsed < rates.decay.length {
                        self.level = exponential(
                            LEVEL_ONE,
                            rates.sustain,
                            position(self.elapsed, rates.decay),
                        );
                        break;
                    }
                    self.elapsed -= rates.decay.length;
                    self.stage = Stage::Sustain;
                }
                Stage::Sustain => {
                    self.level = rates.sustain;
                    break;
                }
                Stage::Release => {
                    self.level = if self.elapsed < rates.release.length {
                        exponential(self.release_from, 0, position(self.elapsed, rates.release))
                    } else {
                        0
                    };
                    break;
                }
            }
        }
        if self.elapsed < ELAPSED_LIMIT {
            self.elapsed += HALF_SAMPLES_PER_SAMPLE;
        }
    }
}

// ---------------------------------------------------------------------------
// Wave tables

#[derive(Clone)]
struct WaveTables {
    tables: [[i8; 256]; WAVE_COUNT],
    user_samples: [[u8; USER_WAVE_LENGTH]; USER_WAVE_COUNT],
}

impl Default for WaveTables {
    fn default() -> Self {
        let mut waves = Self {
            tables: WAVES,
            user_samples: [[0; USER_WAVE_LENGTH]; USER_WAVE_COUNT],
        };
        for index in 0..USER_WAVE_COUNT {
            waves.rebuild_user_wave(index);
        }
        waves
    }
}

impl WaveTables {
    fn set_user_sample(&mut self, index: usize, position: usize, value: u8) {
        self.user_samples[index][position] = value;
        self.rebuild_user_wave(index);
    }

    fn rebuild_user_wave(&mut self, index: usize) {
        let samples = self.user_samples[index];
        let table = &mut self.tables[FIRST_USER_WAVE + index];
        for (i, entry) in table.iter_mut().enumerate() {
            let pre = i32::from(samples[(i / 8) % USER_WAVE_LENGTH]);
            let next = i32::from(samples[(i / 8 + 1) % USER_WAVE_LENGTH]);
            let value = (pre * 8 + (next - pre) * (i % 8) as i32) >> 3;
            *entry = (value - 128) as i8;
        }
    }

    fn sample(&self, wave: u8, position: usize) -> i8 {
        self.tables[wave as usize][position]
    }
}

// ---------------------------------------------------------------------------
// Channel

const TURNS_SATURATED: u32 = 32;

fn modulation_to_phase(signal: i64) -> i64 {
    signal * (1i64 << 19)
}

fn feedback_to_phase(history_sum: i32, feedback: u8) -> i64 {
    i64::from(history_sum) * (i64::from(feedback) - 128) * (1i64 << 11)
}

fn table_position(phase: i64) -> usize {
    let position = if phase >= 0 {
        phase >> 24
    } else {
        -((-phase) >> 24)
    };
    (position as u64 & 0xFF) as usize
}

fn phase_step_for(hz: u16, ratio: u64) -> u64 {
    let rate = u64::from(SAMPLE_RATE);
    (u64::from(hz) * ratio * (1u64 << 20) + rate - 1) / rate
}

#[derive(Clone, Copy)]
struct Group {
    stage_count: usize,
    stages: [u8; OPERATOR_COUNT],
}

#[derive(Clone, Copy)]
struct Algorithm {
    multiply_groups: bool,
    direct_phase: bool,
    group_count: usize,
    groups: [Group; 4],
}

const fn group(stages: &[u8]) -> Group {
    let mut out = [0u8; OPERATOR_COUNT];
    let mut i = 0;
    while i < stages.len() {
        out[i] = stages[i];
        i += 1;
    }
    Group {
        stage_count: stages.len(),
        stages: out,
    }
}

const NONE: Group = group(&[]);
const ALL_ADDITIVE: Group = group(&[0xFF]);
const FOUR_MODULATE_FOUR: Group = group(&[0xF0, 0x0F]);
const PAIR_CHAIN: Group = group(&[0xC0, 0x30, 0x0C, 0x03]);
const EIGHT_CHAIN: Group = group(&[0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01]);
const CHAIN_8_TO_5: Group = group(&[0x80, 0x40, 0x20, 0x10]);
const CHAIN_4_TO_1: Group = group(&[0x08, 0x04, 0x02, 0x01]);
const PAIR_2_TO_1: Group = group(&[0x02, 0x01]);
const PAIR_8_TO_7: Group = group(&[0x80, 0x40]);
const PAIR_6_TO_5: Group = group(&[0x20, 0x10]);
const PAIR_4_TO_3: Group = group(&[0x08, 0x04]);

const fn algorithm(multiply_groups: bool, direct_phase: bool, groups: &[Group]) -> Algorithm {
    let mut out = [NONE; 4];
    let mut i = 0;
    while i < groups.len() {
        out[i] = groups[i];
        i += 1;
    }
    Algorithm {
        multiply_groups,
        direct_phase,
        group_count: groups.len(),
        groups: out,
    }
}

const ALGORITHMS: [Algorithm; ALGORITHM_COUNT as usize] = [
    algorithm(false, false, &[ALL_ADDITIVE]),
    algorithm(false, false, &[FOUR_MODULATE_FOUR]),
    algorithm(true, false, &[group(&[0x0F]), group(&[0xF0])]),
    algorithm(false, false, &[PAIR_CHAIN]),
    algorithm(false, false, &[EIGHT_CHAIN]),
    algorithm(false, false, &[CHAIN_8_TO_5, CHAIN_4_TO_1]),
    algorithm(
        false,
        false,
        &[PAIR_2_TO_1, PAIR_8_TO_7, PAIR_6_TO_5, PAIR_4_TO_3],
    ),
    algorithm(true, false, &[CHAIN_8_TO_5, CHAIN_4_TO_1]),
    algorithm(
        true,
        false,
        &[
            group(&[0x03]),
            group(&[0x0C]),
            group(&[0x30]),
            group(&[0xC0]),
        ],
    ),
    algorithm(
        true,
        false,
        &[PAIR_2_TO_1, PAIR_8_TO_7, PAIR_6_TO_5, PAIR_4_TO_3],
    ),
    algorithm(false, true, &[FOUR_MODULATE_FOUR]),
    algorithm(false, true, &[PAIR_CHAIN]),
    algorithm(false, true, &[EIGHT_CHAIN]),
];

#[derive(Clone, Copy, Default)]
struct OperatorParams {
    ratio: u16,
    volume: u8,
    wave: u8,
    envelope: EnvelopeRegisters,
}

#[derive(Clone, Copy, Default)]
struct Operator {
    envelope: Envelope,
    rates: EnvelopeRates,
    phase_step: u64,
    phase: u32,
    turns: u32,
    history: [i32; 2],
    amplitude: i32,
}

#[derive(Clone, Copy)]
struct Channel {
    frequency: u16,
    algorithm: u8,
    feedback: u8,
    feedback_target: u8,
    muted: bool,
    operators: [OperatorParams; OPERATOR_COUNT],
    ops: [Operator; OPERATOR_COUNT],
    pan_gain_left: i64,
    pan_gain_right: i64,
    gate_on: bool,
}

impl Default for Channel {
    fn default() -> Self {
        let mut channel = Self {
            frequency: 0,
            algorithm: 0,
            feedback: 0,
            feedback_target: 0,
            muted: false,
            operators: [OperatorParams::default(); OPERATOR_COUNT],
            ops: [Operator::default(); OPERATOR_COUNT],
            pan_gain_left: 15,
            pan_gain_right: 15,
            gate_on: false,
        };
        for op in 0..OPERATOR_COUNT {
            channel.set_operator_envelope(op, EnvelopeRegisters::default());
        }
        channel.set_pan(0, 0);
        channel.update_phase_steps();
        channel
    }
}

impl Channel {
    fn set_frequency(&mut self, hz: u16) {
        self.frequency = hz;
        self.update_phase_steps();
    }

    fn set_operator_ratio(&mut self, op: usize, ratio: u16) {
        self.operators[op].ratio = ratio;
        self.update_phase_steps();
    }

    fn set_operator_envelope(&mut self, op: usize, envelope: EnvelopeRegisters) {
        self.operators[op].envelope = envelope;
        self.ops[op].rates = EnvelopeRates::from_registers(envelope);
    }

    fn set_pan(&mut self, left: u8, right: u8) {
        let left = left & 0x0F;
        let right = right & 0x0F;
        let centered = left == 0 && right == 0;
        self.pan_gain_left = if centered { 15 } else { i64::from(left) };
        self.pan_gain_right = if centered { 15 } else { i64::from(right) };
    }

    fn key_on(&mut self) {
        for op in &mut self.ops {
            op.envelope.restart();
            op.phase = 0;
            op.turns = 0;
            op.history = [0, 0];
        }
        self.gate_on = true;
    }

    fn key_off(&mut self) {
        if !self.gate_on {
            return;
        }
        for op in &mut self.ops {
            op.envelope.release();
        }
        self.gate_on = false;
    }

    fn update_phase_steps(&mut self) {
        for op in 0..OPERATOR_COUNT {
            let ratio = if op == 0 {
                RATIO_ONE
            } else {
                u64::from(self.operators[op].ratio)
            };
            self.ops[op].phase_step = phase_step_for(self.frequency, ratio);
        }
    }

    fn tick(&mut self, waves: &WaveTables) -> i32 {
        for index in 0..OPERATOR_COUNT {
            let volume = i32::from(self.operators[index].volume);
            let op = &mut self.ops[index];
            op.amplitude = (op.envelope.level * volume) >> 16;
            let rates = op.rates;
            op.envelope.advance(&rates);
            let next = u64::from(op.phase) + op.phase_step;
            op.phase = next as u32;
            if op.turns < TURNS_SATURATED {
                op.turns =
                    (u64::from(op.turns) + (next >> 32)).min(u64::from(TURNS_SATURATED)) as u32;
            }
        }
        self.render_algorithm(waves)
    }

    fn evaluate_operator(&mut self, index: usize, mut phase: i64, waves: &WaveTables) -> i32 {
        let feedback = self.feedback;
        let wave = self.operators[index].wave;
        let is_target = usize::from(self.feedback_target) == index;
        let op = &mut self.ops[index];
        if is_target {
            phase += feedback_to_phase(op.history[0] + op.history[1], feedback);
        }
        let output = i32::from(waves.sample(wave, table_position(phase))) * op.amplitude;
        op.history[1] = op.history[0];
        op.history[0] = output;
        output
    }

    fn render_algorithm(&mut self, waves: &WaveTables) -> i32 {
        if self.algorithm >= ALGORITHM_COUNT {
            return 0;
        }
        let algorithm = ALGORITHMS[self.algorithm as usize];
        let mut combined: i64 = 0;
        for g in 0..algorithm.group_count {
            let group = algorithm.groups[g];
            let mut modulation: i64 = 0;
            let mut stage_sum: i32 = 0;
            for s in 0..group.stage_count {
                stage_sum = 0;
                for op in 0..OPERATOR_COUNT {
                    if group.stages[s] & (1u8 << op) == 0 {
                        continue;
                    }
                    let source = self.ops[op];
                    let mut phase = (i64::from(source.turns) << 32) | i64::from(source.phase);
                    if s > 0 {
                        phase = if algorithm.direct_phase {
                            modulation
                        } else {
                            phase + modulation
                        };
                    }
                    stage_sum += self.evaluate_operator(op, phase, waves);
                }
                modulation = modulation_to_phase(i64::from(stage_sum));
            }
            if g == 0 {
                combined = i64::from(stage_sum);
            } else if algorithm.multiply_groups {
                combined = (combined * i64::from(stage_sum)) >> SIGNAL_FRAC_BITS;
            } else {
                combined += i64::from(stage_sum);
            }
        }
        combined as i32
    }
}

// ---------------------------------------------------------------------------
// Engine and register interface

const DAC_GAIN: i64 = 2159 * 32767;
const DAC_SHIFT: u32 = 32;

fn to_dac_units(weighted: i64) -> i32 {
    let scaled = weighted * DAC_GAIN;
    (if scaled >= 0 {
        scaled >> DAC_SHIFT
    } else {
        -((-scaled) >> DAC_SHIFT)
    }) as i32
}

fn saturate(value: i32) -> i16 {
    value.clamp(-32768, 32767) as i16
}

/// The register file and engine of one 3HS816.
#[derive(Clone)]
pub struct Chip {
    regs: [u8; REGISTER_SIZE as usize],
    channels: [Channel; CHANNEL_COUNT],
    waves: WaveTables,
}

impl Default for Chip {
    fn default() -> Self {
        let mut regs = [0u8; REGISTER_SIZE as usize];
        regs[ID_BASE as usize..].copy_from_slice(&ID);
        Self {
            regs,
            channels: [Channel::default(); CHANNEL_COUNT],
            waves: WaveTables::default(),
        }
    }
}

impl Chip {
    /// Last written value; the ID is read-only. Outside the chip reads 0.
    pub fn read(&self, address: u32) -> u8 {
        self.regs.get(address as usize).copied().unwrap_or(0)
    }

    pub fn write(&mut self, address: u32, value: u8) {
        if address >= ID_BASE {
            return;
        }
        self.regs[address as usize] = value;
        if address < USER_WAVE_BASE {
            let offset = address % CHANNEL_STRIDE;
            let base = (address - offset) as usize;
            let mut block = [0u8; CHANNEL_STRIDE as usize];
            block.copy_from_slice(&self.regs[base..base + CHANNEL_STRIDE as usize]);
            let channel = &mut self.channels[(address / CHANNEL_STRIDE) as usize];
            write_channel_register(channel, &block, offset, value);
        } else if address < MUTE_BASE {
            let offset = (address - USER_WAVE_BASE) as usize;
            self.waves
                .set_user_sample(offset / USER_WAVE_LENGTH, offset % USER_WAVE_LENGTH, value);
        } else if address < MUTE_BASE + CHANNEL_COUNT as u32 {
            self.channels[(address - MUTE_BASE) as usize].muted = value != 0;
        } else if (FEEDBACK_TARGET_BASE..FEEDBACK_TARGET_BASE + CHANNEL_COUNT as u32)
            .contains(&address)
        {
            self.channels[(address - FEEDBACK_TARGET_BASE) as usize].feedback_target = value;
        }
    }

    /// One 48 kHz stereo sample.
    pub fn sample(&mut self) -> [i16; 2] {
        let mut left: i64 = 0;
        let mut right: i64 = 0;
        for channel in &mut self.channels {
            let raw = i64::from(channel.tick(&self.waves));
            if !channel.muted {
                left += raw * channel.pan_gain_left;
                right += raw * channel.pan_gain_right;
            }
        }
        [saturate(to_dac_units(left)), saturate(to_dac_units(right))]
    }
}

fn write_channel_register(channel: &mut Channel, block: &[u8; 64], offset: u32, value: u8) {
    let read16 = |at: u32| u16::from(block[at as usize]) << 8 | u16::from(block[at as usize + 1]);
    if offset < REG_RATIO {
        channel.set_frequency(read16(0));
    } else if offset < REG_VOLUME {
        channel.set_operator_ratio((offset / 2) as usize, read16(offset - offset % 2));
    } else if offset < REG_WAVE {
        channel.operators[(offset - REG_VOLUME) as usize].volume = value;
    } else if offset < REG_ALGORITHM {
        let op = ((offset - REG_WAVE) * 2) as usize;
        channel.operators[op].wave = value >> 4;
        channel.operators[op + 1].wave = value & 0x0F;
    } else if offset == REG_ALGORITHM {
        channel.algorithm = value;
    } else if offset == REG_PAN {
        channel.set_pan(value >> 4, value & 0x0F);
    } else if offset == REG_GATE {
        if value == 1 {
            channel.key_on();
        } else if value == 0 {
            channel.key_off();
        }
    } else if offset == REG_FEEDBACK {
        channel.feedback = value;
    } else {
        let op = ((offset - REG_ENVELOPE) / 4) as usize;
        let at = (REG_ENVELOPE as usize) + op * 4;
        channel.set_operator_envelope(
            op,
            EnvelopeRegisters {
                attack: block[at],
                decay: block[at + 1],
                sustain: block[at + 2],
                release: block[at + 3],
            },
        );
    }
}
