//! SQV4 version 2 encoder and version 1/2 decoder.

use std::fmt;

const HEADER_SIZE: usize = 16;
const ENCODED_VERSION: u8 = 2;
const FRAME_SAMPLES: usize = 64;
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
        if self == Self::H {
            4
        } else {
            3
        }
    }

    pub(crate) fn payload_bytes(self) -> usize {
        if self == Self::H {
            32
        } else {
            24
        }
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

#[derive(Clone, Copy, Default)]
pub(crate) struct State {
    pub(crate) predictor: i32,
    pub(crate) index: usize,
}

pub(crate) struct Header {
    pub(crate) version: u8,
    pub(crate) variant: Variant,
    pub(crate) channels: u8,
    pub(crate) sample_rate: u32,
    pub(crate) samples_per_channel: usize,
    pub(crate) frame_count: usize,
    pub(crate) frame_size: usize,
}

/// Encode interleaved signed 16-bit PCM into an SQV4 version 2 file.
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
    let frame_count = samples_per_channel.div_ceil(FRAME_SAMPLES);
    let frame_size = 1usize
        .checked_add(variant.payload_bytes())
        .ok_or(Error::SizeOverflow)?;
    let payload_len = frame_count
        .checked_mul(channels as usize)
        .and_then(|v| v.checked_mul(frame_size))
        .ok_or(Error::SizeOverflow)?;
    let file_len = HEADER_SIZE
        .checked_add(payload_len)
        .ok_or(Error::SizeOverflow)?;

    let mut output = Vec::with_capacity(file_len);
    output.extend_from_slice(b"SQV4");
    output.extend_from_slice(&[ENCODED_VERSION, variant as u8, channels, 0]);
    output.extend_from_slice(&sample_rate.to_le_bytes());
    output.extend_from_slice(&samples_u32.to_le_bytes());

    let mut states = [State::default(); 2];
    for frame in 0..frame_count {
        for channel in 0..channels as usize {
            output.push(volume);
            let mut packed = [0u8; 32];
            let first = frame * FRAME_SAMPLES;
            for within in 0..FRAME_SAMPLES {
                let sample_index = first + within;
                let code = if sample_index < samples_per_channel {
                    let input = pcm[sample_index * channels as usize + channel] as i32;
                    let code = encode_code(variant, &mut states[channel], input);
                    code
                } else {
                    0
                };
                pack_code(&mut packed, within, variant.bits(), code);
            }
            output.extend_from_slice(&packed[..variant.payload_bytes()]);
        }
    }
    debug_assert_eq!(output.len(), file_len);
    Ok(output)
}

/// Validate metadata, exact length, and canonical final-fragment padding without decoding.
pub(crate) fn inspect(data: &[u8]) -> Result<Header, Error> {
    if data.len() < HEADER_SIZE {
        return Err(Error::TruncatedHeader);
    }
    if &data[..4] != b"SQV4" {
        return Err(Error::InvalidMagic);
    }
    if data[4] != 1 && data[4] != 2 {
        return Err(Error::UnsupportedVersion(data[4]));
    }
    let version = data[4];
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
    let frame_count = samples_per_channel.div_ceil(FRAME_SAMPLES);
    let frame_size = 1usize
        .checked_add(variant.payload_bytes())
        .ok_or(Error::SizeOverflow)?;
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
    let remainder = samples_per_channel % FRAME_SAMPLES;
    if remainder != 0 {
        let final_frame_offset = HEADER_SIZE + (frame_count - 1) * channels as usize * frame_size;
        for channel in 0..channels as usize {
            let payload_offset = final_frame_offset + channel * frame_size + 1;
            let payload = &data[payload_offset..payload_offset + variant.payload_bytes()];
            if (remainder..FRAME_SAMPLES)
                .any(|sample| unpack_code(payload, sample, variant.bits()) != 0)
            {
                return Err(Error::NonZeroPadding);
            }
        }
    }
    Ok(Header {
        version,
        variant,
        channels,
        sample_rate,
        samples_per_channel,
        frame_count,
        frame_size,
    })
}

/// Decode an entire SQV4 version 1 file. Truncation and trailing data are errors.
pub fn decode(data: &[u8]) -> Result<DecodedAudio, Error> {
    let header = inspect(data)?;
    let output_len = header
        .samples_per_channel
        .checked_mul(header.channels as usize)
        .ok_or(Error::SizeOverflow)?;
    let mut samples = vec![0; output_len];
    let mut states = [State::default(); 2];
    let mut offset = HEADER_SIZE;
    for frame in 0..header.frame_count {
        let count = (header.samples_per_channel - frame * FRAME_SAMPLES).min(FRAME_SAMPLES);
        for channel in 0..header.channels as usize {
            let volume = data[offset];
            let payload = &data[offset + 1..offset + header.frame_size];
            for within in 0..count {
                let code = unpack_code(payload, within, header.variant.bits());
                let reconstructed =
                    decode_code(header.version, header.variant, &mut states[channel], code);
                let gained = reconstructed * volume as i32 / 255;
                let interleaved_index =
                    (frame * FRAME_SAMPLES + within) * header.channels as usize + channel;
                samples[interleaved_index] = gained as i16;
            }
            offset += header.frame_size;
        }
    }
    Ok(DecodedAudio {
        variant: header.variant,
        sample_rate: header.sample_rate,
        channels: header.channels,
        samples,
    })
}

fn pack_code(output: &mut [u8; 32], sample: usize, bits: usize, code: u8) {
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
            if code & 8 != 0 {
                -diff
            } else {
                diff
            }
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
            if code & 4 != 0 {
                -diff
            } else {
                diff
            }
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

pub(crate) fn decode_code(version: u8, variant: Variant, state: &mut State, code: u8) -> i32 {
    update(version, variant, state, code)
}

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
        update(ENCODED_VERSION, variant, state, code);
        return code;
    }
    let mut best_code = 0;
    let mut best_error = i32::MAX;
    for code in 0..8u8 {
        let candidate = (state.predictor + delta(ENCODED_VERSION, variant, *state, code))
            .clamp(i16::MIN as i32, i16::MAX as i32);
        let error = (input - candidate).abs();
        if error < best_error {
            best_error = error;
            best_code = code;
        }
    }
    update(ENCODED_VERSION, variant, state, best_code);
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
    if code & 4 != 0 {
        -diff
    } else {
        diff
    }
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

    #[test]
    fn golden_vector_is_stable() {
        let input = [0i16, 1000, -1000, 32767, -32768];
        let actual = encode_interleaved(&input, 1, 8_000, Variant::H, 255).unwrap();
        let mut expected = [0u8; 49];
        expected[..16].copy_from_slice(&[
            0x53, 0x51, 0x56, 0x34, 2, 0, 1, 0, 0x40, 0x1f, 0, 0, 5, 0, 0, 0,
        ]);
        expected[16..].copy_from_slice(&[
            255, 0x70, 0x7f, 0x0f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn round_trip_all_variants_mono_stereo_padding_and_empty() {
        for variant in [Variant::H, Variant::L, Variant::LPlus] {
            for channels in [1, 2] {
                let pcm: Vec<i16> = (0..131 * channels as usize)
                    .map(|i| if i % channels as usize == 0 { 4 } else { -4 })
                    .collect();
                let bytes = encode_interleaved(&pcm, channels, 22_050, variant, 255).unwrap();
                let decoded = decode(&bytes).unwrap();
                assert_eq!(decoded.variant, variant);
                assert_eq!(decoded.sample_rate, 22_050);
                assert_eq!(decoded.channels, channels);
                assert_eq!(decoded.samples.len(), pcm.len());
                assert!(decoded
                    .samples
                    .iter()
                    .zip(&pcm)
                    .all(|(a, b)| (*a as i32 - *b as i32).abs() < 128));
                let empty = encode_interleaved(&[], channels, 48_000, variant, 10).unwrap();
                assert_eq!(empty.len(), HEADER_SIZE);
                assert!(decode(&empty).unwrap().samples.is_empty());
            }
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
        let valid = encode_interleaved(&[1], 1, 1, Variant::H, 255).unwrap();
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
        let mut bad_padding = valid.clone();
        bad_padding[18] |= 0x10;
        assert_eq!(decode(&bad_padding), Err(Error::NonZeroPadding));
        let mut bad = valid;
        bad[7] = 1;
        assert_eq!(decode(&bad), Err(Error::InvalidReservedByte(1)));
    }

    #[test]
    fn l_plus_codebook_benchmark_is_reproducible() {
        assert_eq!(benchmark_l_plus(), (vec![1, 2, 3, 14], 279_448_221_728));
    }
    #[test]
    fn l_plus_v2_reconstructs_mixed_tone_audio_and_preserves_v1_decoding() {
        let pcm: Vec<i16> = (0..8192)
            .map(|i| L_PLUS_BENCH_TONE[i % L_PLUS_BENCH_TONE.len()])
            .collect();
        let encoded = encode_interleaved(&pcm, 1, 8_000, Variant::LPlus, 255).unwrap();
        assert_eq!(encoded[4], 2);
        let decoded = decode(&encoded).unwrap();
        let squared_error: u64 = pcm
            .iter()
            .zip(&decoded.samples)
            .map(|(&input, &output)| {
                let error = input as i64 - output as i64;
                (error * error) as u64
            })
            .sum();
        assert!(
            squared_error < 32_768_000_000,
            "L+ v2 tone RMSE exceeds 2000"
        );

        let mut legacy = vec![0u8; 41];
        legacy[..4].copy_from_slice(b"SQV4");
        legacy[4..8].copy_from_slice(&[1, 2, 1, 0]);
        legacy[8..12].copy_from_slice(&8_000u32.to_le_bytes());
        legacy[12..16].copy_from_slice(&1u32.to_le_bytes());
        legacy[16] = 255;
        assert_eq!(decode(&legacy).unwrap().samples, [9]);
        legacy[4] = 2;
        assert_eq!(decode(&legacy).unwrap().samples, [1]);
    }

    #[test]
    fn l_plus_v2_matches_shared_golden_codes() {
        let golden: Vec<u8> = include_str!("../../../../s3w2_core/tests/sqv4_lplus_golden.inc")
            .split(',')
            .filter_map(|byte| {
                let byte = byte.trim().trim_start_matches("0x");
                (!byte.is_empty()).then(|| u8::from_str_radix(byte, 16).unwrap())
            })
            .collect();
        let input = [12, 35, 75, 147, 275];
        assert_eq!(
            encode_interleaved(&input, 1, 8_000, Variant::LPlus, 255).unwrap(),
            golden
        );
        assert_eq!(decode(&golden).unwrap().samples, input);
    }
    #[test]
    fn shared_golden_fixture_matches_rust_encoder() {
        let golden: Vec<u8> = include_str!("../../../../s3w2_core/tests/sqv4_golden.inc")
            .split(',')
            .filter_map(|byte| {
                let byte = byte.trim().trim_start_matches("0x");
                (!byte.is_empty()).then(|| u8::from_str_radix(byte, 16).unwrap())
            })
            .collect();
        let input: Vec<i16> = (1..=65).collect();
        assert_eq!(
            encode_interleaved(&input, 1, 8_000, Variant::H, 255).unwrap(),
            golden
        );
        let decoded = decode(&golden).unwrap();
        assert_eq!(decoded.variant, Variant::H);
        assert_eq!(decoded.sample_rate, 8_000);
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.samples, input);
        assert_eq!((golden[16], golden[49]), (255, 255));
        let mut reduced_volume = golden.clone();
        reduced_volume[16] = 127;
        reduced_volume[49] = 127;
        let expected: Vec<i16> = input
            .iter()
            .map(|sample| (*sample as i32 * 127 / 255) as i16)
            .collect();
        assert_eq!(decode(&reduced_volume).unwrap().samples, expected);
    }
}
