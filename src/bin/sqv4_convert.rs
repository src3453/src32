use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use cpt32::devices::sgu::sqv4::{self, Variant};

const USAGE: &str = "usage: sqv4_convert INPUT.wav OUTPUT.sqv|OUTPUT.sqv4 [H|L|L+] [VOLUME 0..255]";

struct PcmWave {
    channels: u8,
    sample_rate: u32,
    samples: Vec<i16>,
}

fn read_pcm16_wave(data: &[u8]) -> Result<PcmWave, String> {
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

fn write_pcm16_wave(
    path: &Path,
    channels: u8,
    sample_rate: u32,
    samples: &[i16],
) -> Result<(), String> {
    if channels != 1 && channels != 2 {
        return Err("preview channel count must be mono or stereo".into());
    }
    let data_size = samples
        .len()
        .checked_mul(2)
        .ok_or("preview WAV data size overflow")?;
    let data_size_u32 =
        u32::try_from(data_size).map_err(|_| "preview WAV exceeds RIFF size limit")?;
    let riff_size = 36u32
        .checked_add(data_size_u32)
        .ok_or("preview RIFF size overflow")?;
    let block_align = channels as u16 * 2;
    let byte_rate = sample_rate
        .checked_mul(block_align as u32)
        .ok_or("preview WAV byte rate overflow")?;

    let mut wav = Vec::with_capacity(44 + data_size);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_size.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&(channels as u16).to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size_u32.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, wav).map_err(|error| format!("failed to write preview WAV: {error}"))
}

fn parse_variant(text: &str) -> Result<Variant, String> {
    match text.to_ascii_uppercase().as_str() {
        "H" => Ok(Variant::H),
        "L" => Ok(Variant::L),
        "L+" | "LPLUS" => Ok(Variant::LPlus),
        _ => Err(format!("invalid variant {text:?}; expected H, L, or L+")),
    }
}

fn preview_path(output: &Path) -> Result<PathBuf, String> {
    let stem = output.file_stem().ok_or("output path has no file name")?;
    let mut name = stem.to_os_string();
    name.push(".preview.wav");
    Ok(output.with_file_name(name))
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 2 || args.len() > 4 || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Err(USAGE.into());
    }
    let input_path = PathBuf::from(&args[0]);
    let output_path = PathBuf::from(&args[1]);
    let extension = output_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if !extension.eq_ignore_ascii_case("sqv") && !extension.eq_ignore_ascii_case("sqv4") {
        return Err("output extension must be .sqv or .sqv4".into());
    }
    let variant = args
        .get(2)
        .map(|text| parse_variant(text))
        .transpose()?
        .unwrap_or(Variant::H);
    let volume = args
        .get(3)
        .map(|text| {
            text.parse::<u8>()
                .map_err(|_| "volume must be an integer from 0 to 255".to_string())
        })
        .transpose()?
        .unwrap_or(255);

    let input_bytes =
        fs::read(&input_path).map_err(|error| format!("failed to read input WAV: {error}"))?;
    let input = read_pcm16_wave(&input_bytes)?;
    let encoded = sqv4::encode_interleaved(
        &input.samples,
        input.channels,
        input.sample_rate,
        variant,
        volume,
    )
    .map_err(|error| error.to_string())?;
    let preview = sqv4::decode(&encoded).map_err(|error| error.to_string())?;
    let preview_path = preview_path(&output_path)?;
    if input_path.canonicalize().ok() == preview_path.canonicalize().ok() {
        return Err("preview path would overwrite the input WAV".into());
    }

    fs::write(&output_path, encoded)
        .map_err(|error| format!("failed to write SQV4 output: {error}"))?;
    write_pcm16_wave(
        &preview_path,
        preview.channels,
        preview.sample_rate,
        &preview.samples,
    )?;
    println!("SQV4: {}", output_path.display());
    println!("Preview WAV: {}", preview_path.display());
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        process::exit(2);
    }
}
