// Audio playback output using cpal for SGU (3WS8PN / S3W2)

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};

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

    pub fn push_interleaved(&mut self, samples: &[f32]) {
        let write_pos = self.write_pos.load(Ordering::Relaxed);
        let read_pos = self.read_pos.load(Ordering::Acquire);
        let available = (read_pos + self.capacity - write_pos - 1) % self.capacity;

        if samples.len() > available {
            // Buffer overflow - skip old samples
            let skip = samples.len() - available;
            let new_read = (read_pos + skip) % self.capacity;
            self.read_pos.store(new_read, Ordering::Release);
        }

        let mut cur = write_pos;
        for &sample in samples {
            self.buffer[cur] = sample;
            cur = (cur + 1) % self.capacity;
        }
        self.write_pos.store(cur, Ordering::Release);
    }

    pub fn pop(&mut self) -> Option<f32> {
        let read_pos = self.read_pos.load(Ordering::Relaxed);
        let write_pos = self.write_pos.load(Ordering::Acquire);

        if read_pos == write_pos {
            None
        } else {
            let sample = self.buffer[read_pos];
            self.read_pos.store((read_pos + 1) % self.capacity, Ordering::Release);
            Some(sample)
        }
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
                .map_err(|e| format!("Failed to get default output config: {e}"))?
        };

        let sample_rate = chosen_config.sample_rate();
        let channels = chosen_config.channels() as usize;
        let sample_format = chosen_config.sample_format();
        let config: StreamConfig = chosen_config.into();

        // 48000Hz * 2 channels * 0.2 sec buffer = ~19200 samples
        let buffer_capacity = (sample_rate as usize * channels).max(8192);
        let ring_buffer = Arc::new(Mutex::new(AudioRingBuffer::new(buffer_capacity)));

        let stream_buf = Arc::clone(&ring_buffer);
        let err_fn = |err| eprintln!("Audio stream error: {}", err);

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

    /// Push stereo i16 samples (left, right interleaved or from slices)
    pub fn push_samples_i16(&self, left: &[i16], right: &[i16]) {
        let count = left.len().min(right.len());
        let mut interleaved = Vec::with_capacity(count * 2);
        for i in 0..count {
            interleaved.push(left[i] as f32 / 32768.0);
            interleaved.push(right[i] as f32 / 32768.0);
        }

        if let Ok(mut buf) = self.ring_buffer.lock() {
            buf.push_interleaved(&interleaved);
        }
    }
}
