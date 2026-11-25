//! `MediaError` — structured domain errors for the media crate.

use std::path::PathBuf;

/// All errors that can be produced by the media crate.
#[derive(Debug)]
pub enum MediaError {
    /// The file could not be opened for reading.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Symphonia failed to probe or demux the file.
    Probe(String),
    /// No audio track was found in the file.
    NoAudioTrack,
    /// The codec is not supported.
    UnsupportedCodec(String),
    /// A decoding error occurred on a specific packet.
    DecodeFailed(String),
    /// The resampler received invalid parameters.
    InvalidResamplerParams { from_rate: u32, to_rate: u32 },
    /// The asset could not be found in the manager.
    AssetNotFound(esperia_project_model::audio_source::SourceId),
    /// The worker channel is disconnected (manager was dropped).
    WorkerDisconnected,
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "I/O error on {}: {source}", path.display()),
            Self::Probe(s) => write!(f, "probe failed: {s}"),
            Self::NoAudioTrack => write!(f, "file contains no audio track"),
            Self::UnsupportedCodec(c) => write!(f, "unsupported codec: {c}"),
            Self::DecodeFailed(s) => write!(f, "decode error: {s}"),
            Self::InvalidResamplerParams { from_rate, to_rate } => {
                write!(f, "invalid resample rates: {from_rate} → {to_rate}")
            }
            Self::AssetNotFound(id) => write!(f, "asset not found: {id:?}"),
            Self::WorkerDisconnected => write!(f, "asset manager worker is disconnected"),
        }
    }
}

impl std::error::Error for MediaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
