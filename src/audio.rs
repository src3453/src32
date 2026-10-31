// Audio playback output using cpal for SGU (3WS8PN / S3W2)

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Common frame-based audio contract for every guest audio producer.
pub trait AudioSource {
    fn render_audio(&mut self, sample_rate: u32, left: &mut [i16], right: &mut [i16]);
}

/// Render, meter and optionally add one source to the output mix.
pub fn mix_source(
    source: &mut impl AudioSource,
    sample_rate: u32,
    muted: bool,
    mix_left: &mut [i16],
    mix_right: &mut [i16],
) -> (f32, f32) {
    let samples = mix_left.len().min(mix_right.len());
    let mut left = vec![0; samples];
    let mut right = vec![0; samples];
    source.render_audio(sample_rate, &mut left, &mut right);
    let mut sum = 0.0f64;
    let mut peak = 0.0f32;
    for (&l, &r) in left.iter().zip(&right) {
        for sample in [l, r] {
            let value = sample as f32 / 32768.0;
            sum += f64::from(value * value);
            peak = peak.max(value.abs());
        }
    }
    if !muted {
        for i in 0..samples {
            mix_left[i] = mix_left[i].saturating_add(left[i]);
            mix_right[i] = mix_right[i].saturating_add(right[i]);
        }
    }
    ((sum / (samples * 2).max(1) as f64).sqrt() as f32, peak)
}

pub struct AudioRingBuffer {
    buffer: Vec<f32>,
    capacity: usize,
    read_pos: AtomicUsize,
    write_pos: AtomicUsize,
}

impl AudioRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![0.0; capacity],
            capacity,
            read_pos: AtomicUsize::new(0),
            write_pos: AtomicUsize::new(0),
        }
    }

    pub fn push_stereo_i16(&mut self, left: &[i16], right: &[i16]) {
        let frames = left.len().min(right.len());
        let sample_count = frames * 2;
        let write_pos = self.write_pos.load(Ordering::Relaxed);
        let read_pos = self.read_pos.load(Ordering::Acquire);
        let available = (read_pos + self.capacity - write_pos - 1) % self.capacity;

        if sample_count > available {
            // Buffer overflow - skip old samples
            let skip = sample_count - available;
            let new_read = (read_pos + skip) % self.capacity;
            self.read_pos.store(new_read, Ordering::Release);
        }

        let mut cur = write_pos;
        for i in 0..frames {
            self.buffer[cur] = left[i] as f32 / 32768.0;
            cur = (cur + 1) % self.capacity;
            self.buffer[cur] = right[i] as f32 / 32768.0;
            cur = (cur + 1) % self.capacity;
        }
        self.write_pos.store(cur, Ordering::Release);
    }

    pub fn pop(&mut self) -> Option<f32> {
        let read_pos = self.read_pos.load(Ordering::Relaxed);
        let write_pos = self.write_pos.load(Ordering::Acquire);
        if read_pos == write_pos {
            return None;
        }
        let sample = self.buffer[read_pos];
        self.read_pos
            .store((read_pos + 1) % self.capacity, Ordering::Release);
        Some(sample)
    }

    pub fn fill_slice(&mut self, output: &mut [f32]) {
        for s in output.iter_mut() {
            *s = self.pop().unwrap_or(0.0);
        }
    }
}

pub struct AudioHost {
    _stream: Stream,
    ring_buffer: Arc<Mutex<AudioRingBuffer>>,
    sample_rate: u32,
}

impl AudioHost {
    pub fn new() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No audio output device found".to_string())?;

        let supported_configs_range = device
            .supported_output_configs()
            .map_err(|e| format!("Failed to get supported configs: {e}"))?;

        // Find a stereo 48000Hz config if possible, or best match
        let mut chosen_config = None;
        for cfg in supported_configs_range {
            if cfg.channels() == 2 && cfg.sample_format() == SampleFormat::F32 {
                if cfg.min_sample_rate() <= 48000 && cfg.max_sample_rate() >= 48000 {
                    chosen_config = Some(cfg.with_sample_rate(48000));
                    break;
                }
            }
        }

        let chosen_config = if let Some(cfg) = chosen_config {
            cfg
        } else {
            device
                .default_output_config()
                .map_err(|e| format!("Failed to get default config: {e}"))?
        };

        let sample_rate = chosen_config.sample_rate();
        let channels = chosen_config.channels() as usize;
        let sample_format = chosen_config.sample_format();
        let config: StreamConfig = chosen_config.into();

        // 48000Hz * 2 channels * 0.2 sec buffer = ~19200 samples
        let buffer_capacity = (sample_rate as usize * channels).max(8192);
        let ring_buffer = Arc::new(Mutex::new(AudioRingBuffer::new(buffer_capacity)));

        let stream_buf = Arc::clone(&ring_buffer);
        let err_fn = |err| tracing::error!(%err, "Audio stream error");

        let stream = match sample_format {
            SampleFormat::F32 => device.build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if let Ok(mut buf) = stream_buf.lock() {
                        buf.fill_slice(data);
                    } else {
                        data.fill(0.0);
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_output_stream(
                config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    if let Ok(mut buf) = stream_buf.lock() {
                        for s in data.iter_mut() {
                            let f = buf.pop().unwrap_or(0.0);
                            *s = (f * 32767.0).clamp(-32768.0, 32767.0) as i16;
                        }
                    } else {
                        data.fill(0);
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::U16 => device.build_output_stream(
                config,
                move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                    if let Ok(mut buf) = stream_buf.lock() {
                        for s in data.iter_mut() {
                            let f = buf.pop().unwrap_or(0.0);
                            *s = ((f * 32767.0).clamp(-32768.0, 32767.0) as i32 + 32768) as u16;
                        }
                    } else {
                        data.fill(32768);
                    }
                },
                err_fn,
                None,
            ),
            _ => return Err("Unsupported sample format".to_string()),
        }
        .map_err(|e| format!("Failed to build output stream: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("Failed to play stream: {e}"))?;

        Ok(Self {
            _stream: stream,
            ring_buffer,
            sample_rate,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Push stereo i16 samples (left/right), interleaved into the host ring buffer.
    pub fn push_samples_i16(&self, left: &[i16], right: &[i16]) {
        if let Ok(mut buf) = self.ring_buffer.lock() {
            buf.push_stereo_i16(left, right);
        }
    }
}

/// Read the optional SGU v3 host input device name from the emulator configuration.
#[allow(dead_code)]
pub fn input_device_from_config(path: &std::path::Path) -> Result<Option<String>, String> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    input_device_from_yaml(&contents)
}

#[allow(dead_code)]
fn input_device_from_yaml(contents: &str) -> Result<Option<String>, String> {
    let root: yaml_serde::Value =
        yaml_serde::from_str(contents).map_err(|e| format!("invalid YAML: {e}"))?;
    let Some(audio) = root.get("audio") else {
        return Ok(None);
    };
    let table = audio.as_mapping().ok_or("`audio` must be a mapping")?;
    let Some(value) = table.get("input_device") else {
        return Ok(None);
    };
    let name = value
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or("`audio.input_device` must be a nonempty device name string")?;
    Ok(Some(name.to_owned()))
}

/// Captured input frames awaiting the guest, capped to bound the latency.
type CaptureQueue = Arc<Mutex<std::collections::VecDeque<(i16, i16)>>>;

/// The host's default recording device as the 34SI22 ADC: 48 kHz stereo,
/// mono duplicated to both sides, other rates resampled (zero-order hold).
#[allow(dead_code)] // music_editor includes this file but has no 34SI22
pub struct AudioInput {
    _stream: Stream,
    shared: CaptureQueue,
    pending: std::collections::VecDeque<(i16, i16)>,
    device_rate: u32,
    phase: u64,
    last: (i16, i16),
}

#[allow(dead_code)]
impl AudioInput {
    pub fn new(device_name: Option<&str>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = if let Some(name) = device_name {
            let mut matches = host
                .input_devices()
                .map_err(|e| format!("Failed to enumerate audio input devices: {e}"))?
                .filter(|device| device.description().is_ok_and(|d| d.name() == name));
            let device = matches
                .next()
                .ok_or_else(|| format!("Audio input device not found: {name}"))?;
            if matches.next().is_some() {
                return Err(format!("Multiple audio input devices named: {name}"));
            }
            device
        } else {
            host.default_input_device()
                .ok_or_else(|| "No audio input device found".to_string())?
        };
        tracing::info!(device = ?device.description(), "34SI22 audio input selected");
        let supported = device
            .default_input_config()
            .map_err(|e| format!("Failed to get input config: {e}"))?;
        let device_rate = supported.sample_rate();
        let channels = supported.channels().max(1) as usize;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let shared: CaptureQueue = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let cap = (device_rate as usize / 4).max(1024);

        fn push(queue: &CaptureQueue, cap: usize, channels: usize, data: &[f32]) {
            let Ok(mut queue) = queue.lock() else { return };
            for frame in data.chunks(channels) {
                let l = (frame[0] * 32767.0).clamp(-32768.0, 32767.0) as i16;
                let r = frame
                    .get(1)
                    .map_or(l, |r| (r * 32767.0).clamp(-32768.0, 32767.0) as i16);
                queue.push_back((l, r));
            }
            while queue.len() > cap {
                queue.pop_front();
            }
        }

        let err_fn = |err| tracing::error!(%err, "Audio input stream error");
        let q = Arc::clone(&shared);
        let stream = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| push(&q, cap, channels, data),
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let f: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                    push(&q, cap, channels, &f);
                },
                err_fn,
                None,
            ),
            SampleFormat::U16 => device.build_input_stream(
                config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    let f: Vec<f32> = data
                        .iter()
                        .map(|&s| (s as i32 - 32768) as f32 / 32768.0)
                        .collect();
                    push(&q, cap, channels, &f);
                },
                err_fn,
                None,
            ),
            _ => return Err("Unsupported input sample format".to_string()),
        }
        .map_err(|e| format!("Failed to build input stream: {e}"))?;
        stream
            .play()
            .map_err(|e| format!("Failed to start input stream: {e}"))?;
        Ok(Self {
            _stream: stream,
            shared,
            pending: std::collections::VecDeque::new(),
            device_rate,
            phase: 0,
            last: (0, 0),
        })
    }
}

impl cpt32::devices::sgu::sgu::AdcSource for AudioInput {
    /// One 48 kHz frame: consume `device_rate / 48000` captured frames and hold the
    /// last one; an empty queue (underrun) holds the previous value.
    fn next_frame(&mut self) -> (i16, i16) {
        self.phase += u64::from(self.device_rate);
        while self.phase >= 48_000 {
            self.phase -= 48_000;
            if self.pending.is_empty() {
                if let Ok(mut shared) = self.shared.lock() {
                    self.pending.extend(shared.drain(..));
                }
            }
            if let Some(frame) = self.pending.pop_front() {
                self.last = frame;
            }
        }
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_device_config_validates_name() {
        assert_eq!(super::input_device_from_yaml("{}").unwrap(), None);
        assert_eq!(super::input_device_from_yaml("audio: {}\n").unwrap(), None);
        assert_eq!(
            super::input_device_from_yaml("audio:\n  input_device: 'Voicemeeter Out A1'\n")
                .unwrap()
                .as_deref(),
            Some("Voicemeeter Out A1")
        );
        for yaml in [
            "audio: 1",
            "audio: {input_device: 42}",
            "audio: {input_device: ''}",
            "audio: {input_device: null}",
        ] {
            assert!(super::input_device_from_yaml(yaml).is_err(), "{yaml}");
        }
    }

    #[test]
    fn stereo_push_preserves_channel_order_and_sample_values() {
        let mut buffer = AudioRingBuffer::new(8);
        buffer.push_stereo_i16(&[i16::MAX, i16::MIN], &[0, 16_384]);

        let mut output = [0.0; 4];
        buffer.fill_slice(&mut output);
        assert_eq!(output, [i16::MAX as f32 / 32768.0, 0.0, -1.0, 0.5]);
    }
}
