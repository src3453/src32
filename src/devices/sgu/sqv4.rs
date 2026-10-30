//! SQV4 version 3 encoder and version 1/2/3 decoder
//! (doc/squeezvox/squeezvox4_spec_stub.md).
//!
//! A version 3 channel fragment carries its own gain and initial step index and
//! starts from predictor 0, so it decodes without any earlier fragment. The
//! encoder normalizes each fragment to full scale before quantizing. Versions 1
//! and 2 (64-sample fragments, predictor state carried through the file) are
//! decoded for SGU v1/v2/v2.1 playback and old files only.

use std::fmt;

const HEADER_SIZE: usize = 16;
/// The version every encoder writes.
pub(crate) const VERSION: u8 = 3;
/// Samples per version 3 channel fragment.
pub(crate) const FRAGMENT_SAMPLES: usize = 23;
/// Version 3 volume byte and step-index byte that precede the packed codes.
pub(crate) const FRAGMENT_PREFIX: usize = 2;
/// Samples per version 1/2 channel fragment.
pub(crate) const LEGACY_FRAGMENT_SAMPLES: usize = 64;
pub(crate) const MAX_STEP_INDEX: u8 = 88;
const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const H_INDEX_ADJUST: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];
const L_INDEX_ADJUST: [i32; 4] = [-1, 2, 4, 6];
const LEGACY_L_PLUS_LEVELS: [i32; 4] = [10, 11, 12, 14];
const L_PLUS_LEVELS: [i32; 4] = [1, 2, 3, 14];
const L_LEVELS: [i32; 4] = [1, 3, 5, 7];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Variant {
    H = 0,
    L = 1,
    LPlus = 2,
}

impl TryFrom<u8> for Variant {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::H),
            1 => Ok(Self::L),
            2 => Ok(Self::LPlus),
            _ => Err(Error::InvalidVariant(value)),
        }
    }
}

impl Variant {
    pub(crate) fn bits(self) -> usize {
        if self == Self::H { 4 } else { 3 }
    }

    /// Version 3 packed code bytes: 23 codes rounded up to whole bytes (H 12, L/L+ 9).
    pub(crate) fn payload_bytes(self) -> usize {
        (FRAGMENT_SAMPLES * self.bits()).div_ceil(8)
    }

    /// Version 3 channel fragment: volume, step index, packed codes (H 14, L/L+ 11).
    pub(crate) fn fragment_size(self) -> usize {
        FRAGMENT_PREFIX + self.payload_bytes()
    }

    /// Version 1/2 channel fragment: volume and 64 packed codes (H 33, L/L+ 25).
    fn legacy_fragment_size(self) -> usize {
        1 + LEGACY_FRAGMENT_SAMPLES * self.bits() / 8
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidChannels(u8),
    ZeroSampleRate,
    MisalignedInterleavedSamples,
    TooManySamples,
    InvalidVariant(u8),
    TruncatedHeader,
    InvalidMagic,
    UnsupportedVersion(u8),
    InvalidReservedByte(u8),
    SizeOverflow,
    InvalidFileLength { expected: usize, actual: usize },
    InvalidStepIndex(u8),
    NonZeroPadding,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SQV4 error: {self:?}")
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedAudio {
    pub variant: Variant,
    pub sample_rate: u32,
    pub channels: u8,
    /// Interleaved signed PCM samples.
    pub samples: Vec<i16>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct State {
    pub(crate) predictor: i32,
    pub(crate) index: usize,
}

impl State {
    /// Version 3 decoder state at the start of a fragment whose index byte is
    /// `index`. Out-of-range bytes saturate to 88 (validated files never contain them).
    pub(crate) fn fragment_start(index: u8) -> Self {
        Self {
            predictor: 0,
            index: index.min(MAX_STEP_INDEX) as usize,
        }
    }
}

pub(crate) struct Header {
    pub(crate) version: u8,
    pub(crate) variant: Variant,
    pub(crate) channels: u8,
    pub(crate) sample_rate: u32,
    pub(crate) samples_per_channel: usize,
    /// Samples per channel fragment: 23 for version 3, 64 for versions 1/2.
    pub(crate) frame_samples: usize,
    pub(crate) frame_count: usize,
    pub(crate) frame_size: usize,
}

/// Encode interleaved signed 16-bit PCM into an SQV4 version 3 file.
///
/// `volume` is a linear pre-gain (`x * volume / 255`, toward zero) applied before
/// each fragment is normalized; 255 encodes the input unchanged.
pub fn encode_interleaved(
    pcm: &[i16],
    channels: u8,
    sample_rate: u32,
    variant: Variant,
    volume: u8,
) -> Result<Vec<u8>, Error> {
    if channels != 1 && channels != 2 {
        return Err(Error::InvalidChannels(channels));
    }
    if sample_rate == 0 {
        return Err(Error::ZeroSampleRate);
    }
    if pcm.len() % channels as usize != 0 {
        return Err(Error::MisalignedInterleavedSamples);
    }
    let samples_per_channel = pcm.len() / channels as usize;
    let samples_u32 = u32::try_from(samples_per_channel).map_err(|_| Error::TooManySamples)?;
    let frame_count = samples_per_channel.div_ceil(FRAGMENT_SAMPLES);
    let file_len = frame_count
        .checked_mul(channels as usize)
        .and_then(|v| v.checked_mul(variant.fragment_size()))
        .and_then(|v| v.checked_add(HEADER_SIZE))
        .ok_or(Error::SizeOverflow)?;

    let mut output = Vec::with_capacity(file_len);
    output.extend_from_slice(&header_bytes(variant, channels, sample_rate, samples_u32));
    let mut fragment = [0i16; FRAGMENT_SAMPLES];
    for frame in 0..frame_count {
        let first = frame * FRAGMENT_SAMPLES;
        let count = (samples_per_channel - first).min(FRAGMENT_SAMPLES);
        for channel in 0..channels as usize {
            for (within, slot) in fragment[..count].iter_mut().enumerate() {
                let input = pcm[(first + within) * channels as usize + channel] as i32;
                *slot = (input * volume as i32 / 255) as i16;
            }
            encode_fragment(variant, &fragment[..count], &mut output);
        }
    }
    debug_assert_eq!(output.len(), file_len);
    Ok(output)
}

/// The 16-byte version 3 header.
pub(crate) fn header_bytes(
    variant: Variant,
    channels: u8,
    sample_rate: u32,
    samples_per_channel: u32,
) -> [u8; HEADER_SIZE] {
    let mut header = [0u8; HEADER_SIZE];
    header[..4].copy_from_slice(b"SQV4");
    header[4..8].copy_from_slice(&[VERSION, variant as u8, channels, 0]);
    header[8..12].copy_from_slice(&sample_rate.to_le_bytes());
    header[12..16].copy_from_slice(&samples_per_channel.to_le_bytes());
    header
}

/// Fragment volume: the smallest gain whose full scale covers the peak magnitude.
pub(crate) fn fragment_volume(samples: &[i16]) -> u8 {
    let peak = samples
        .iter()
        .map(|&sample| (sample as i32).unsigned_abs())
        .max()
        .unwrap_or(0);
    (peak * 255).div_ceil(32767).min(255) as u8
}

/// Scale a sample to the fragment's full scale: `x * 255 / volume`, rounded to
/// nearest with ties away from zero, clamped to 16 bits.
fn normalize(sample: i16, volume: u8) -> i32 {
    let numerator = sample as i32 * 255;
    let volume = volume as i32;
    let rounded = (numerator.abs() + volume / 2) / volume;
    (if numerator < 0 { -rounded } else { rounded }).clamp(i16::MIN as i32, i16::MAX as i32)
}

/// Encode 1..=23 samples of one channel as a complete version 3 fragment
/// appended to `output`.
pub(crate) fn encode_fragment(variant: Variant, samples: &[i16], output: &mut Vec<u8>) {
    encode_fragment_with_volume(variant, samples, fragment_volume(samples), output);
}

/// Encode a fragment at a given volume: the samples are normalized by it and the
/// initial step index minimizing the squared output error is chosen (ties: lowest).
fn encode_fragment_with_volume(
    variant: Variant,
    samples: &[i16],
    volume: u8,
    output: &mut Vec<u8>,
) {
    debug_assert!(!samples.is_empty() && samples.len() <= FRAGMENT_SAMPLES);
    let mut codes = [0u8; FRAGMENT_SAMPLES];
    let mut best_index = 0u8;
    if volume != 0 {
        let mut normalized = [0i32; FRAGMENT_SAMPLES];
        for (slot, &sample) in normalized.iter_mut().zip(samples) {
            *slot = normalize(sample, volume);
        }
        let mut best_error = u64::MAX;
        let mut trial = [0u8; FRAGMENT_SAMPLES];
        for index in 0..=MAX_STEP_INDEX {
            let mut state = State::fragment_start(index);
            let mut error = 0u64;
            for (within, &input) in samples.iter().enumerate() {
                let code = encode_code(variant, &mut state, normalized[within]);
                trial[within] = code;
                let diff = input as i64 - apply_volume(state.predictor, volume as i32) as i64;
                error += (diff * diff) as u64;
            }
            if error < best_error {
                best_error = error;
                best_index = index;
                codes = trial;
            }
        }
    }
    output.push(volume);
    output.push(best_index);
    let mut packed = [0u8; 12];
    for (within, &code) in codes[..samples.len()].iter().enumerate() {
        pack_code(&mut packed, within, variant.bits(), code);
    }
    output.extend_from_slice(&packed[..variant.payload_bytes()]);
}

/// Version 3 output gain: `reconstructed * volume / 255`, rounded to nearest with
/// ties away from zero (the inverse of `normalize`).
pub(crate) fn apply_volume(reconstructed: i32, volume: i32) -> i32 {
    let product = reconstructed * volume;
    let rounded = (product.abs() + 127) / 255;
    if product < 0 { -rounded } else { rounded }
}

/// Decode the first `out.len()` samples of one complete version 3 channel fragment.
pub(crate) fn decode_fragment(variant: Variant, fragment: &[u8], out: &mut [i16]) {
    let volume = fragment[0] as i32;
    let mut state = State::fragment_start(fragment[1]);
    let payload = &fragment[FRAGMENT_PREFIX..];
    for (within, slot) in out.iter_mut().enumerate() {
        let code = unpack_code(payload, within, variant.bits());
        *slot = apply_volume(decode_code(VERSION, variant, &mut state, code), volume) as i16;
    }
}

/// Validate metadata, exact length, step indexes, and zero padding without decoding.
pub(crate) fn inspect(data: &[u8]) -> Result<Header, Error> {
    if data.len() < HEADER_SIZE {
        return Err(Error::TruncatedHeader);
    }
    if &data[..4] != b"SQV4" {
        return Err(Error::InvalidMagic);
    }
    let version = data[4];
    if !(1..=VERSION).contains(&version) {
        return Err(Error::UnsupportedVersion(version));
    }
    let variant = Variant::try_from(data[5])?;
    let channels = data[6];
    if channels != 1 && channels != 2 {
        return Err(Error::InvalidChannels(channels));
    }
    if data[7] != 0 {
        return Err(Error::InvalidReservedByte(data[7]));
    }
    let sample_rate = u32::from_le_bytes(data[8..12].try_into().unwrap());
    if sample_rate == 0 {
        return Err(Error::ZeroSampleRate);
    }
    let samples_per_channel = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
    let (frame_samples, frame_size, prefix) = if version == VERSION {
        (FRAGMENT_SAMPLES, variant.fragment_size(), FRAGMENT_PREFIX)
    } else {
        (LEGACY_FRAGMENT_SAMPLES, variant.legacy_fragment_size(), 1)
    };
    let frame_count = samples_per_channel.div_ceil(frame_samples);
    let expected = frame_count
        .checked_mul(channels as usize)
        .and_then(|v| v.checked_mul(frame_size))
        .and_then(|v| v.checked_add(HEADER_SIZE))
        .ok_or(Error::SizeOverflow)?;
    if data.len() != expected {
        return Err(Error::InvalidFileLength {
            expected,
            actual: data.len(),
        });
    }
    let bits = variant.bits();
    for (position, fragment) in data[HEADER_SIZE..].chunks_exact(frame_size).enumerate() {
        if version == VERSION && fragment[1] > MAX_STEP_INDEX {
            return Err(Error::InvalidStepIndex(fragment[1]));
        }
        // Code slots past the last sample and the bits that round the payload up
        // to a byte are zero.
        let frame = position / channels as usize;
        let count = (samples_per_channel - frame * frame_samples).min(frame_samples);
        let payload = &fragment[prefix..];
        let used_bits = count * bits;
        let partial = used_bits % 8;
        let whole = used_bits / 8;
        let tail = if partial != 0 { whole + 1 } else { whole };
        if (partial != 0 && payload[whole] >> partial != 0)
            || payload[tail..].iter().any(|&byte| byte != 0)
        {
            return Err(Error::NonZeroPadding);
        }
    }
    Ok(Header {
        version,
        variant,
        channels,
        sample_rate,
        samples_per_channel,
        frame_samples,
        frame_count,
        frame_size,
    })
}

/// Decode an entire SQV4 version 1, 2, or 3 file. Truncation and trailing data are errors.
pub fn decode(data: &[u8]) -> Result<DecodedAudio, Error> {
    let header = inspect(data)?;
    let channels = header.channels as usize;
    let output_len = header
        .samples_per_channel
        .checked_mul(channels)
        .ok_or(Error::SizeOverflow)?;
    let mut samples = vec![0; output_len];
    let mut legacy_states = [State::default(); 2];
    let mut fragment_samples = [0i16; LEGACY_FRAGMENT_SAMPLES];
    for (position, fragment) in data[HEADER_SIZE..]
        .chunks_exact(header.frame_size)
        .enumerate()
    {
        let (frame, channel) = (position / channels, position % channels);
        let first = frame * header.frame_samples;
        let count = (header.samples_per_channel - first).min(header.frame_samples);
        let out = &mut fragment_samples[..count];
        if header.version == VERSION {
            decode_fragment(header.variant, fragment, out);
        } else {
            decode_legacy_fragment(
                header.version,
                header.variant,
                &mut legacy_states[channel],
                fragment,
                out,
            );
        }
        for (within, &sample) in out.iter().enumerate() {
            samples[(first + within) * channels + channel] = sample;
        }
    }
    Ok(DecodedAudio {
        variant: header.variant,
        sample_rate: header.sample_rate,
        channels: header.channels,
        samples,
    })
}

/// Decode the first `out.len()` samples of a version 1/2 fragment, continuing
/// from the channel's carried predictor state.
pub(crate) fn decode_legacy_fragment(
    version: u8,
    variant: Variant,
    state: &mut State,
    fragment: &[u8],
    out: &mut [i16],
) {
    let volume = fragment[0] as i32;
    for (within, slot) in out.iter_mut().enumerate() {
        let code = unpack_code(&fragment[1..], within, variant.bits());
        *slot = (decode_code(version, variant, state, code) * volume / 255) as i16;
    }
}

fn pack_code(output: &mut [u8; 12], sample: usize, bits: usize, code: u8) {
    let bit_offset = sample * bits;
    for bit in 0..bits {
        if (code & (1 << bit)) != 0 {
            output[(bit_offset + bit) / 8] |= 1 << ((bit_offset + bit) % 8);
        }
    }
}

pub(crate) fn unpack_code(input: &[u8], sample: usize, bits: usize) -> u8 {
    let bit_offset = sample * bits;
    let mut code = 0;
    for bit in 0..bits {
        code |= ((input[(bit_offset + bit) / 8] >> ((bit_offset + bit) % 8)) & 1) << bit;
    }
    code
}

fn delta(version: u8, variant: Variant, state: State, code: u8) -> i32 {
    let step = STEPS[state.index];
    match variant {
        Variant::H => {
            let magnitude = (code & 7) as i32;
            let diff = (step >> 3)
                + if magnitude & 1 != 0 { step >> 2 } else { 0 }
                + if magnitude & 2 != 0 { step >> 1 } else { 0 }
                + if magnitude & 4 != 0 { step } else { 0 };
            if code & 8 != 0 { -diff } else { diff }
        }
        Variant::L | Variant::LPlus => {
            let magnitude = (code & 3) as usize;
            let level = match variant {
                Variant::L => L_LEVELS[magnitude],
                Variant::LPlus if version == 1 => LEGACY_L_PLUS_LEVELS[magnitude],
                Variant::LPlus => L_PLUS_LEVELS[magnitude],
                Variant::H => unreachable!(),
            };
            let diff = (level * step + 4) / 8;
            if code & 4 != 0 { -diff } else { diff }
        }
    }
}

fn update(version: u8, variant: Variant, state: &mut State, code: u8) -> i32 {
    let difference = delta(version, variant, *state, code);
    state.predictor = (state.predictor + difference).clamp(i16::MIN as i32, i16::MAX as i32);
    let adjustment = match variant {
        Variant::H => H_INDEX_ADJUST[code as usize],
        Variant::L | Variant::LPlus => L_INDEX_ADJUST[(code & 3) as usize],
    };
    state.index = (state.index as i32 + adjustment).clamp(0, 88) as usize;
    state.predictor
}

/// Reconstructed (pre-volume) predictor after `code`.
pub(crate) fn decode_code(version: u8, variant: Variant, state: &mut State, code: u8) -> i32 {
    update(version, variant, state, code)
}

/// Select the code for a normalized `input` and advance `state` with it.
fn encode_code(variant: Variant, state: &mut State, input: i32) -> u8 {
    if variant == Variant::H {
        let step = STEPS[state.index];
        let difference = (input - state.predictor).abs();
        let mut remaining = difference;
        let mut magnitude = 0;
        for (bit, threshold) in [(4, step), (2, step >> 1), (1, step >> 2)] {
            if remaining >= threshold {
                magnitude |= bit;
                remaining -= threshold;
            }
        }
        let code = magnitude | if input < state.predictor { 8 } else { 0 };
        update(VERSION, variant, state, code);
        return code;
    }
    let mut best_code = 0;
    let mut best_error = i32::MAX;
    for code in 0..8u8 {
        let candidate = (state.predictor + delta(VERSION, variant, *state, code))
            .clamp(i16::MIN as i32, i16::MAX as i32);
        let error = (input - candidate).abs();
        if error < best_error {
            best_error = error;
            best_code = code;
        }
    }
    update(VERSION, variant, state, best_code);
    best_code
}

#[cfg(test)]
const L_PLUS_BENCH_TONE: [i16; 128] = [
    0, 8926, 15150, 17259, 15693, 12291, 9111, 7209, 6123, 4321, 355, -5996, -13304, -19056,
    -20923, -17999, -11314, -3305, 3432, 7423, 8889, 9316, 10311, 12445, 14782, 15391, 12542, 5883,
    -3121, -11761, -17368, -18600, -16000, -11529, -7368, -4690, -3121, -1188, 2542, 8320, 14782,
    19516, 20311, 16387, 8889, 352, -6568, -10376, -11314, -10928, -10923, -11985, -13304, -13067,
    -9645, -2750, 6123, 14280, 19111, 19362, 15693, 10188, 5150, 1855, 0, -1855, -5150, -10188,
    -15693, -19362, -19111, -14280, -6123, 2750, 9645, 13067, 13304, 11985, 10923, 10928, 11314,
    10376, 6568, -352, -8889, -16387, -20311, -19516, -14782, -8320, -2542, 1188, 3121, 4690, 7368,
    11529, 16000, 18600, 17368, 11761, 3121, -5883, -12542, -15391, -14782, -12445, -10311, -9316,
    -8889, -7423, -3432, 3305, 11314, 17999, 20923, 19056, 13304, 5996, -355, -4321, -6123, -7209,
    -9111, -12291, -15693, -17259, -15150, -8926,
];

/// Exhaustively evaluate the specified L+ codebooks with deterministic tonal fixtures.
#[cfg(test)]
fn benchmark_l_plus() -> (Vec<i32>, u128) {
    let mut fixtures = Vec::with_capacity(6);
    fixtures.push(vec![0i16; 1024]);
    let mut impulse = vec![0i16; 1024];
    impulse[0] = 32767;
    impulse[511] = -32768;
    fixtures.push(impulse);
    fixtures.push(
        (0..1024)
            .map(|i| (-32768 + (i % 256) as i32 * 256) as i16)
            .collect(),
    );
    fixtures.push(
        (0..1024)
            .map(|i| (-32768i64 + (i as i64 * 65535 / 1023)) as i16)
            .collect(),
    );
    fixtures.push(
        (0..8192)
            .map(|i| L_PLUS_BENCH_TONE[i % L_PLUS_BENCH_TONE.len()])
            .collect(),
    );
    fixtures.push(
        (0..1024)
            .map(|i| if i % 128 < 64 { 24576 } else { -24576 })
            .collect(),
    );

    let mut winner = Vec::new();
    let mut best_score = u128::MAX;
    for a in 1..=12 {
        for b in (a + 1)..=13 {
            for c in (b + 1)..=14 {
                for d in (c + 1)..=15 {
                    if b - a == c - b && c - b == d - c {
                        continue;
                    }
                    let levels = [a, b, c, d];
                    let mut score = 0u128;
                    for fixture in &fixtures {
                        let mut encoder = State::default();
                        let mut decoder = State::default();
                        for &sample in fixture {
                            let code = encode_code_with_levels(&mut encoder, sample as i32, levels);
                            let decoded = decode_code_with_levels(&mut decoder, code, levels);
                            let error = sample as i64 - decoded as i64;
                            score += (error * error) as u128;
                        }
                    }
                    if score < best_score {
                        best_score = score;
                        winner = levels.to_vec();
                    }
                }
            }
        }
    }
    (winner, best_score)
}

#[cfg(test)]
fn encode_code_with_levels(state: &mut State, input: i32, levels: [i32; 4]) -> u8 {
    let mut best = 0;
    let mut best_error = i32::MAX;
    for code in 0..8u8 {
        let candidate = (state.predictor + delta_with_levels(*state, code, levels))
            .clamp(i16::MIN as i32, i16::MAX as i32);
        let error = (input - candidate).abs();
        if error < best_error {
            best_error = error;
            best = code;
        }
    }
    update_with_levels(state, best, levels);
    best
}

#[cfg(test)]
fn decode_code_with_levels(state: &mut State, code: u8, levels: [i32; 4]) -> i32 {
    update_with_levels(state, code, levels)
}

#[cfg(test)]
fn delta_with_levels(state: State, code: u8, levels: [i32; 4]) -> i32 {
    let diff = (levels[(code & 3) as usize] * STEPS[state.index] + 4) / 8;
    if code & 4 != 0 { -diff } else { diff }
}

#[cfg(test)]
fn update_with_levels(state: &mut State, code: u8, levels: [i32; 4]) -> i32 {
    let difference = delta_with_levels(*state, code, levels);
    state.predictor = (state.predictor + difference).clamp(i16::MIN as i32, i16::MAX as i32);
    state.index = (state.index as i32 + L_INDEX_ADJUST[(code & 3) as usize]).clamp(0, 88) as usize;
    state.predictor
}

#[cfg(test)]
mod tests {
    use super::*;

    fn squared_error(a: &[i16], b: &[i16]) -> u64 {
        a.iter()
            .zip(b)
            .map(|(&x, &y)| {
                let e = x as i64 - y as i64;
                (e * e) as u64
            })
            .sum()
    }

    #[test]
    fn version_3_fragment_layout_is_stable() {
        let input = [0i16, 1000, -1000, 32767, -32768];
        let actual = encode_interleaved(&input, 1, 8_000, Variant::H, 255).unwrap();
        assert_eq!(actual.len(), HEADER_SIZE + 14);
        assert_eq!(
            &actual[..16],
            &[
                0x53, 0x51, 0x56, 0x34, 3, 0, 1, 0, 0x40, 0x1f, 0, 0, 5, 0, 0, 0
            ]
        );
        // Peak 32768 saturates the volume; 5 codes use 20 bits, the rest is zero.
        assert_eq!(actual[16], 255);
        assert!(actual[17] <= MAX_STEP_INDEX);
        assert!(actual[18 + 2] >> 4 == 0 && actual[18 + 3..].iter().all(|&b| b == 0));
        assert_eq!(
            [Variant::H, Variant::L, Variant::LPlus].map(Variant::fragment_size),
            [14, 11, 11]
        );
    }

    #[test]
    fn volume_is_the_ceiling_of_the_peak_magnitude() {
        assert_eq!(fragment_volume(&[0, 0]), 0);
        assert_eq!(fragment_volume(&[1]), 1);
        assert_eq!(fragment_volume(&[128, -129]), 2);
        assert_eq!(fragment_volume(&[-16384]), 128);
        assert_eq!(fragment_volume(&[32767]), 255);
        assert_eq!(fragment_volume(&[-32768]), 255);
        // A silent fragment is volume 0, index 0, zero codes.
        let silent = encode_interleaved(&[0; 23], 1, 8_000, Variant::L, 255).unwrap();
        assert!(silent[16..].iter().all(|&b| b == 0));
        assert_eq!(decode(&silent).unwrap().samples, [0; 23]);
    }

    #[test]
    fn round_trip_all_variants_mono_stereo_padding_and_empty() {
        for variant in [Variant::H, Variant::L, Variant::LPlus] {
            for channels in [1, 2] {
                let pcm: Vec<i16> = (0..131 * channels as usize)
                    .map(|i| if i % channels as usize == 0 { 4 } else { -4 })
                    .collect();
                let bytes = encode_interleaved(&pcm, channels, 22_050, variant, 255).unwrap();
                assert_eq!(
                    bytes.len(),
                    HEADER_SIZE + 6 * channels as usize * variant.fragment_size()
                );
                let decoded = decode(&bytes).unwrap();
                assert_eq!(decoded.variant, variant);
                assert_eq!(decoded.sample_rate, 22_050);
                assert_eq!(decoded.channels, channels);
                assert!(
                    decoded
                        .samples
                        .iter()
                        .zip(&pcm)
                        .all(|(a, b)| (*a as i32 - *b as i32).abs() <= 1),
                    "{variant:?} {channels}ch: {:?}",
                    &decoded.samples[..8]
                );
                let empty = encode_interleaved(&[], channels, 48_000, variant, 10).unwrap();
                assert_eq!(empty.len(), HEADER_SIZE);
                assert!(decode(&empty).unwrap().samples.is_empty());
            }
        }
    }

    #[test]
    fn pre_gain_scales_the_input_before_normalization() {
        let pcm: Vec<i16> = (0..46).map(|i| (i * 500 - 11000) as i16).collect();
        let half: Vec<i16> = pcm.iter().map(|&x| (x as i32 * 128 / 255) as i16).collect();
        assert_eq!(
            encode_interleaved(&pcm, 1, 8_000, Variant::H, 128).unwrap(),
            encode_interleaved(&half, 1, 8_000, Variant::H, 255).unwrap()
        );
    }

    #[test]
    fn normalization_reduces_quantization_error_for_quiet_fragments() {
        // Measured RMS (920 samples): /8 L 171->136, L+ 182->132; /2048 H 0.46->0.13.
        // H at moderate levels gains little: its step table is already log-scaled.
        for (variant, divisor) in [
            (Variant::L, 8),
            (Variant::LPlus, 8),
            (Variant::L, 64),
            (Variant::LPlus, 64),
            (Variant::H, 2048),
            (Variant::L, 2048),
            (Variant::LPlus, 2048),
        ] {
            let pcm: Vec<i16> = (0..23 * 40)
                .map(|i| L_PLUS_BENCH_TONE[i % 128] / divisor)
                .collect();
            let decoded = decode(&encode_interleaved(&pcm, 1, 8_000, variant, 255).unwrap())
                .unwrap()
                .samples;
            let mut fixed = header_bytes(variant, 1, 8_000, pcm.len() as u32).to_vec();
            for fragment in pcm.chunks(FRAGMENT_SAMPLES) {
                encode_fragment_with_volume(variant, fragment, 255, &mut fixed);
            }
            let unnormalized = decode(&fixed).unwrap().samples;
            let (normalized, plain) = (
                squared_error(&pcm, &decoded),
                squared_error(&pcm, &unnormalized),
            );
            assert!(
                normalized < plain,
                "{variant:?}/{divisor}: {normalized} vs {plain}"
            );
        }
    }

    #[test]
    fn every_fragment_decodes_on_its_own() {
        let pcm: Vec<i16> = (0..8192)
            .map(|i| L_PLUS_BENCH_TONE[i % 128] / (1 + (i / 700) as i16))
            .collect();
        for variant in [Variant::H, Variant::L, Variant::LPlus] {
            let bytes = encode_interleaved(&pcm, 1, 48_000, variant, 255).unwrap();
            let full = decode(&bytes).unwrap().samples;
            let size = variant.fragment_size();
            // Joining a stream at any fragment reproduces the full decode from there.
            for (frame, fragment) in bytes[HEADER_SIZE..].chunks_exact(size).enumerate() {
                let first = frame * FRAGMENT_SAMPLES;
                let count = (pcm.len() - first).min(FRAGMENT_SAMPLES);
                let mut out = [0i16; FRAGMENT_SAMPLES];
                decode_fragment(variant, fragment, &mut out[..count]);
                assert_eq!(&out[..count], &full[first..first + count]);
            }
            let tail_frames = 100;
            let mut tail = header_bytes(
                variant,
                1,
                48_000,
                (pcm.len() - tail_frames * FRAGMENT_SAMPLES) as u32,
            )
            .to_vec();
            tail.extend_from_slice(&bytes[HEADER_SIZE + tail_frames * size..]);
            assert_eq!(
                decode(&tail).unwrap().samples,
                &full[tail_frames * FRAGMENT_SAMPLES..]
            );
        }
    }

    #[test]
    fn malformed_files_and_encoder_arguments_are_rejected() {
        assert_eq!(
            encode_interleaved(&[1], 2, 1, Variant::H, 255),
            Err(Error::MisalignedInterleavedSamples)
        );
        assert_eq!(
            encode_interleaved(&[], 0, 1, Variant::H, 255),
            Err(Error::InvalidChannels(0))
        );
        assert_eq!(
            encode_interleaved(&[], 1, 0, Variant::H, 255),
            Err(Error::ZeroSampleRate)
        );
        let valid = encode_interleaved(&[1000], 1, 1, Variant::L, 255).unwrap();
        assert_eq!(valid.len(), HEADER_SIZE + 11);
        assert_eq!(decode(&valid[..15]), Err(Error::TruncatedHeader));
        let mut trailing = valid.clone();
        trailing.push(0);
        assert!(matches!(
            decode(&trailing),
            Err(Error::InvalidFileLength { .. })
        ));
        assert!(matches!(
            decode(&valid[..valid.len() - 1]),
            Err(Error::InvalidFileLength { .. })
        ));
        let mut bad_index = valid.clone();
        bad_index[17] = 89;
        assert_eq!(decode(&bad_index), Err(Error::InvalidStepIndex(89)));
        let mut unused_code = valid.clone();
        unused_code[18] |= 0x08; // second code slot of a one-sample fragment
        assert_eq!(decode(&unused_code), Err(Error::NonZeroPadding));
        let mut version = valid.clone();
        version[4] = 4;
        assert_eq!(decode(&version), Err(Error::UnsupportedVersion(4)));
        let mut bad = valid;
        bad[7] = 1;
        assert_eq!(decode(&bad), Err(Error::InvalidReservedByte(1)));

        // Full fragments: the 4 (H) / 3 (L) bits that round 23 codes up to a byte.
        let pcm = [1000i16; 23];
        for (variant, last, mask) in [(Variant::H, 13, 0xF0), (Variant::L, 10, 0xE0)] {
            let mut file = encode_interleaved(&pcm, 1, 1, variant, 255).unwrap();
            assert_eq!(file[HEADER_SIZE + last] & mask, 0);
            file[HEADER_SIZE + last] |= 0x80;
            assert_eq!(decode(&file), Err(Error::NonZeroPadding), "{variant:?}");
        }
    }

    #[test]
    fn l_plus_codebook_benchmark_is_reproducible() {
        assert_eq!(benchmark_l_plus(), (vec![1, 2, 3, 14], 279_448_221_728));
    }

    #[test]
    fn legacy_versions_decode_with_carried_state() {
        // Version 1/2 L+: one sample, code 0, volume 255; only the codebook differs.
        let mut legacy = vec![0u8; 41];
        legacy[..4].copy_from_slice(b"SQV4");
        legacy[4..8].copy_from_slice(&[1, 2, 1, 0]);
        legacy[8..12].copy_from_slice(&8_000u32.to_le_bytes());
        legacy[12..16].copy_from_slice(&1u32.to_le_bytes());
        legacy[16] = 255;
        assert_eq!(decode(&legacy).unwrap().samples, [9]);
        legacy[4] = 2;
        assert_eq!(decode(&legacy).unwrap().samples, [1]);

        // Version 2 H, 65 samples of code 4 (+step): the predictor and index carry
        // from the first 64-sample fragment into the second.
        let mut file = vec![0u8; 16 + 2 * 33];
        file[..16].copy_from_slice(&[
            0x53, 0x51, 0x56, 0x34, 2, 0, 1, 0, 0x40, 0x1f, 0, 0, 65, 0, 0, 0,
        ]);
        file[16] = 255;
        file[17..49].fill(0x44);
        file[49] = 255;
        file[50] = 0x04;
        let samples = decode(&file).unwrap().samples;
        let mut state = State::default();
        let expected: Vec<i16> = (0..65)
            .map(|_| decode_code(2, Variant::H, &mut state, 4) as i16)
            .collect();
        assert_eq!(samples, expected);
        // A fresh state would reconstruct 7 (step 7, code 4) for sample 64.
        assert_eq!(samples[64], i16::MAX);
    }
}
