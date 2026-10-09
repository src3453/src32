//! Shared host-side validation for guest driver stream loading.
use super::stream::{MusicError, StreamPlayer};
use std::path::Path;

pub const STREAM_BASE: u32 = 0x0020_0000;
pub const MAIN_RAM_SIZE: usize = 0x0100_0000;

pub fn read_music_stream(path: &Path, driver_size: usize) -> Result<Vec<u8>, MusicError> {
    if driver_size >= STREAM_BASE as usize {
        return Err(MusicError::new(
            "music driver image must be smaller than 2 MiB",
        ));
    }
    let bytes = std::fs::read(path)?;
    if bytes.len() > MAIN_RAM_SIZE - STREAM_BASE as usize {
        return Err(MusicError::new("music stream exceeds main RAM"));
    }
    StreamPlayer::from_bytes(&bytes)?;
    Ok(bytes)
}
