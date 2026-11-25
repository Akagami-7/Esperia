//! Streaming WAV file writer for the background recording worker.
//!
//! Writes a 44-byte RIFF/WAVE PCM16 header up front, streams sample chunks as
//! they arrive from the ring buffer, and patches the chunk byte lengths on
//! `finalize()`. Never called from the audio callback.

use crate::recording_errors::RecordingError;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Streaming 16-bit PCM WAV writer.
pub struct WavFileWriter {
    path: PathBuf,
    writer: BufWriter<File>,
    sample_rate: u32,
    channels: u16,
    data_bytes_written: u32,
    finalized: bool,
}

impl WavFileWriter {
    /// Create a new WAV file at `path` and write the initial RIFF header.
    pub fn create(path: PathBuf, sample_rate: u32, channels: u16) -> Result<Self, RecordingError> {
        if sample_rate == 0 || channels == 0 {
            return Err(RecordingError::InvalidConfig(format!(
                "invalid WAV params: sr={sample_rate}, ch={channels}"
            )));
        }

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| RecordingError::Io {
                path: path.clone(),
                source: e,
            })?;
        }

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| RecordingError::Io {
                path: path.clone(),
                source: e,
            })?;

        let mut writer = BufWriter::new(file);
        write_wav_header(&mut writer, sample_rate, channels, 0).map_err(|e| {
            RecordingError::Io {
                path: path.clone(),
                source: e,
            }
        })?;

        Ok(Self {
            path,
            writer,
            sample_rate,
            channels,
            data_bytes_written: 0,
            finalized: false,
        })
    }

    /// Encode interleaved `f32` samples as 16-bit signed PCM and write to disk.
    pub fn write_f32_samples(&mut self, samples: &[f32]) -> Result<(), RecordingError> {
        for &s in samples {
            let clamped = s.clamp(-1.0, 1.0);
            let pcm = (clamped * i16::MAX as f32).round() as i16;
            self.writer
                .write_all(&pcm.to_le_bytes())
                .map_err(|e| RecordingError::Io {
                    path: self.path.clone(),
                    source: e,
                })?;
            self.data_bytes_written = self.data_bytes_written.saturating_add(2);
        }
        Ok(())
    }

    /// Flush buffered bytes and patch the RIFF and `data` chunk sizes.
    pub fn finalize(mut self) -> Result<u64, RecordingError> {
        self.finalize_in_place()
    }

    fn finalize_in_place(&mut self) -> Result<u64, RecordingError> {
        if self.finalized {
            return Ok(self.frames_written());
        }
        self.writer.flush().map_err(|e| RecordingError::Io {
            path: self.path.clone(),
            source: e,
        })?;

        let file = self.writer.get_mut();
        // Patch RIFF chunk size at offset 4: 36 + data_bytes_written
        file.seek(SeekFrom::Start(4))
            .map_err(|e| RecordingError::Io {
                path: self.path.clone(),
                source: e,
            })?;
        file.write_all(&36u32.saturating_add(self.data_bytes_written).to_le_bytes())
            .map_err(|e| RecordingError::Io {
                path: self.path.clone(),
                source: e,
            })?;

        // Patch data chunk size at offset 40
        file.seek(SeekFrom::Start(40))
            .map_err(|e| RecordingError::Io {
                path: self.path.clone(),
                source: e,
            })?;
        file.write_all(&self.data_bytes_written.to_le_bytes())
            .map_err(|e| RecordingError::Io {
                path: self.path.clone(),
                source: e,
            })?;
        file.flush().map_err(|e| RecordingError::Io {
            path: self.path.clone(),
            source: e,
        })?;

        self.finalized = true;
        Ok(self.frames_written())
    }

    pub fn frames_written(&self) -> u64 {
        let bytes_per_frame = (self.channels as u64) * 2;
        (self.data_bytes_written as u64)
            .checked_div(bytes_per_frame)
            .unwrap_or(0)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WavFileWriter {
    fn drop(&mut self) {
        if !self.finalized {
            let _ = self.finalize_in_place();
        }
    }
}

/// Write a complete `f32` sample slice as a 16-bit PCM WAV file (used after
/// optional resampling or latency trim).
pub fn write_complete_wav(
    path: &Path,
    samples: &[f32],
    sample_rate: u32,
    channels: u16,
) -> Result<u64, RecordingError> {
    let mut writer = WavFileWriter::create(path.to_path_buf(), sample_rate, channels)?;
    writer.write_f32_samples(samples)?;
    writer.finalize()
}

fn write_wav_header<W: Write>(
    w: &mut W,
    sample_rate: u32,
    channels: u16,
    data_bytes: u32,
) -> std::io::Result<()> {
    let byte_rate = sample_rate * (channels as u32) * 2;
    let block_align = channels * 2;
    w.write_all(b"RIFF")?;
    w.write_all(&36u32.saturating_add(data_bytes).to_le_bytes())?;
    w.write_all(b"WAVE")?;
    w.write_all(b"fmt ")?;
    w.write_all(&16u32.to_le_bytes())?; // PCM fmt chunk size
    w.write_all(&1u16.to_le_bytes())?; // format = PCM
    w.write_all(&channels.to_le_bytes())?;
    w.write_all(&sample_rate.to_le_bytes())?;
    w.write_all(&byte_rate.to_le_bytes())?;
    w.write_all(&block_align.to_le_bytes())?;
    w.write_all(&16u16.to_le_bytes())?; // bits per sample
    w.write_all(b"data")?;
    w.write_all(&data_bytes.to_le_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use esperia_media::decode::decode_audio;

    #[test]
    fn wav_writer_creates_valid_symphonia_readable_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("take1.wav");

        let mut writer = WavFileWriter::create(path.clone(), 48_000, 1).unwrap();
        let samples: Vec<f32> = (0..480)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin() * 0.5)
            .collect();
        writer.write_f32_samples(&samples).unwrap();
        let frames = writer.finalize().unwrap();
        assert_eq!(frames, 480);

        let decoded = decode_audio(&path, None).unwrap();
        assert_eq!(decoded.sample_rate, 48_000);
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.duration_frames, 480);
    }
}
