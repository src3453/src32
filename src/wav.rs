//! RIFF/WAVE decoding shared by PCM asset consumers.

use std::fmt;

/// A malformed or unsupported PCM WAVE file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveError(&'static str);

impl fmt::Display for WaveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for WaveError {}

impl From<&'static str> for WaveError {
    fn from(message: &'static str) -> Self {
        Self(message)
    }
}

/// Interleaved signed 16-bit PCM samples, with one or two channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcmWave {
    pub channels: u8,
    pub sample_rate: u32,
    pub samples: Vec<i16>,
}

/// Decode uncompressed 16-bit mono or stereo WAVE data.
///
/// Sample rates must be nonzero and have a representable PCM byte rate;
/// consumers may impose their own additional rate limits.
pub fn read_pcm16_wave(data: &[u8]) -> Result<PcmWave, WaveError> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("input is not a RIFF/WAVE file".into());
    }
    let riff_size = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let riff_end = riff_size.checked_add(8).ok_or("RIFF size overflow")?;
    if riff_end < 12 || riff_end > data.len() {
        return Err("truncated RIFF/WAVE file".into());
    }

    let mut format = None;
    let mut pcm_data = None;
    let mut offset = 12;
    while offset < riff_end {
        if riff_end - offset < 8 {
            return Err("truncated WAVE chunk header".into());
        }
        let chunk_id = &data[offset..offset + 4];
        let chunk_size =
            u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let chunk_start = offset + 8;
        let chunk_end = chunk_start
            .checked_add(chunk_size)
            .ok_or("WAVE chunk size overflow")?;
        let padded_end = chunk_end
            .checked_add(chunk_size & 1)
            .ok_or("WAVE chunk padding overflow")?;
        if padded_end > riff_end {
            return Err("WAVE chunk extends past RIFF boundary".into());
        }
        if chunk_id == b"fmt " {
            if format.is_some() || chunk_size < 16 {
                return Err("invalid or duplicate WAVE fmt chunk".into());
            }
            let fields = &data[chunk_start..chunk_end];
            format = Some((
                u16::from_le_bytes(fields[0..2].try_into().unwrap()),
                u16::from_le_bytes(fields[2..4].try_into().unwrap()),
                u32::from_le_bytes(fields[4..8].try_into().unwrap()),
                u32::from_le_bytes(fields[8..12].try_into().unwrap()),
                u16::from_le_bytes(fields[12..14].try_into().unwrap()),
                u16::from_le_bytes(fields[14..16].try_into().unwrap()),
            ));
        } else if chunk_id == b"data" {
            if pcm_data.is_some() {
                return Err("duplicate WAVE data chunk".into());
            }
            pcm_data = Some(&data[chunk_start..chunk_end]);
        }
        offset = padded_end;
    }

    let (encoding, channels, sample_rate, byte_rate, block_align, bits_per_sample) =
        format.ok_or("WAVE fmt chunk is missing")?;
    if encoding != 1 || bits_per_sample != 16 {
        return Err("input must use uncompressed 16-bit PCM WAV".into());
    }
    if channels != 1 && channels != 2 {
        return Err("input must be mono or stereo".into());
    }
    if sample_rate == 0 {
        return Err("WAVE sample rate must be nonzero".into());
    }
    let expected_align = channels * 2;
    let expected_rate = sample_rate
        .checked_mul(expected_align as u32)
        .ok_or("WAVE byte rate overflow")?;
    if block_align != expected_align || byte_rate != expected_rate {
        return Err("inconsistent WAVE byte rate or block alignment".into());
    }
    let pcm_data = pcm_data.ok_or("WAVE data chunk is missing")?;
    if pcm_data.len() % block_align as usize != 0 {
        return Err("WAVE data ends in a partial PCM frame".into());
    }
    let samples = pcm_data
        .chunks_exact(2)
        .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
        .collect();
    Ok(PcmWave {
        channels: channels as u8,
        sample_rate,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(channels: u16, sample_rate: u32, samples: &[i16]) -> Vec<u8> {
        let data_size = (samples.len() * 2) as u32;
        let block_align = channels * 2;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
        bytes.extend_from_slice(&block_align.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_size.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn decodes_mono_and_interleaved_stereo() {
        for channels in [1, 2] {
            let samples = [i16::MIN, -1, 0, i16::MAX];
            let decoded = read_pcm16_wave(&wave(channels, 44_100, &samples)).unwrap();
            assert_eq!(decoded.channels, channels as u8);
            assert_eq!(decoded.sample_rate, 44_100);
            assert_eq!(decoded.samples, samples);
        }
    }

    #[test]
    fn retains_converter_sample_rate_allowance() {
        let decoded = read_pcm16_wave(&wave(1, 3_000_000, &[0])).unwrap();
        assert_eq!(decoded.sample_rate, 3_000_000);
        assert_eq!(
            read_pcm16_wave(&wave(1, 0, &[0])).unwrap_err().to_string(),
            "WAVE sample rate must be nonzero"
        );
    }

    #[test]
    fn rejects_partial_stereo_frame() {
        let error = read_pcm16_wave(&wave(2, 44_100, &[1, 2, 3])).unwrap_err();
        assert_eq!(error.to_string(), "WAVE data ends in a partial PCM frame");
    }

    #[test]
    fn rejects_truncated_riff_and_headers() {
        let valid = wave(1, 44_100, &[0]);
        for end in 0..valid.len() {
            assert!(read_pcm16_wave(&valid[..end]).is_err(), "length {end}");
        }
        let mut short_header = valid[..12].to_vec();
        short_header[4..8].copy_from_slice(&5u32.to_le_bytes());
        short_header.push(0);
        assert_eq!(
            read_pcm16_wave(&short_header).unwrap_err().to_string(),
            "truncated WAVE chunk header"
        );
        let mut bad_magic = valid;
        bad_magic[0] = b'X';
        assert!(read_pcm16_wave(&bad_magic).is_err());
    }

    #[test]
    fn rejects_chunks_crossing_declared_riff_boundary() {
        let mut bytes = wave(1, 44_100, &[0]);
        bytes[4..8].copy_from_slice(&36u32.to_le_bytes());
        assert_eq!(
            read_pcm16_wave(&bytes).unwrap_err().to_string(),
            "WAVE chunk extends past RIFF boundary"
        );
    }

    #[test]
    fn requires_odd_chunk_padding_within_riff() {
        let mut bytes = wave(1, 44_100, &[0]);
        bytes.extend_from_slice(b"JUNK");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.push(42);
        let size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&size.to_le_bytes());
        assert_eq!(
            read_pcm16_wave(&bytes).unwrap_err().to_string(),
            "WAVE chunk extends past RIFF boundary"
        );
        bytes.push(0);
        bytes[4..8].copy_from_slice(&(size + 1).to_le_bytes());
        assert_eq!(read_pcm16_wave(&bytes).unwrap().samples, [0]);
    }

    #[test]
    fn rejects_duplicate_chunks_and_invalid_pcm_format() {
        let valid = wave(1, 44_100, &[0]);
        for chunk in [&valid[12..36], &valid[36..]] {
            let mut bytes = valid.clone();
            bytes.extend_from_slice(chunk);
            let size = (bytes.len() - 8) as u32;
            bytes[4..8].copy_from_slice(&size.to_le_bytes());
            assert!(read_pcm16_wave(&bytes).is_err());
        }
        for (offset, value) in [(20, 3u16), (22, 3), (32, 1), (34, 8)] {
            let mut bytes = valid.clone();
            bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
            assert!(read_pcm16_wave(&bytes).is_err());
        }
    }
}
