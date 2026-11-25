//! `AssetManager` — background-thread audio asset loader.
//!
//! # Design
//! * The UI thread calls `load_async()` to enqueue a decode job.
//! * A single background worker thread calls `decode_audio()` sequentially.
//! * When decoding completes, the result is stored behind an `Arc<Mutex<HashMap>>`.
//! * Any thread (including the UI for waveform display) can call `get()` to read
//!   a completed asset.
//! * The audio callback **never** touches `AssetManager` directly.  Instead the UI
//!   thread builds a `ClipSchedule` from the decoded assets and passes it to the
//!   audio engine via an atomic pointer swap (see `esperia-audio-engine`).
//!
//! # Thread safety
//! Sending a `LoadRequest` over the `crossbeam_channel` is the only form of
//! synchronisation between the UI thread and the worker; the result map is
//! locked only briefly for inserts and reads.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};

use crate::decode::{DecodedAudio, decode_audio};
use crate::error::MediaError;
use esperia_project_model::audio_source::SourceId;

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// Snapshot of an asset's load state at the time of calling `status()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadStatus {
    /// A decode job has been queued but has not yet started.
    Pending,
    /// Decoding is in progress.
    Loading,
    /// Decoding completed successfully.
    Ready,
    /// Decoding failed; the string carries the error description.
    Failed(String),
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal types
// ─────────────────────────────────────────────────────────────────────────────

struct LoadRequest {
    id: SourceId,
    path: PathBuf,
    target_sample_rate: Option<u32>,
}

#[derive(Clone)]
enum AssetState {
    Pending,
    Loading,
    Ready(Arc<DecodedAudio>),
    Failed(String),
}

// ─────────────────────────────────────────────────────────────────────────────
// AssetManager
// ─────────────────────────────────────────────────────────────────────────────

/// Background-thread audio asset loader.
///
/// Drop `AssetManager` to shut down the worker thread gracefully.
pub struct AssetManager {
    sender: Sender<LoadRequest>,
    /// Shared state: status + decoded audio per source ID.
    assets: Arc<Mutex<HashMap<SourceId, AssetState>>>,
    /// Join handle for the background thread (kept for clean shutdown).
    _worker: thread::JoinHandle<()>,
}

impl AssetManager {
    /// Create a new manager and start its background worker.
    ///
    /// `target_sample_rate` is forwarded to the decoder; use the engine's
    /// configured rate so that loaded assets are ready to mix without
    /// per-callback resampling.
    pub fn new(target_sample_rate: Option<u32>) -> Self {
        let (tx, rx): (Sender<LoadRequest>, Receiver<LoadRequest>) = bounded(256);
        let assets: Arc<Mutex<HashMap<SourceId, AssetState>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let assets_worker = Arc::clone(&assets);
        let worker = thread::Builder::new()
            .name("esperia-asset-loader".into())
            .spawn(move || worker_loop(rx, assets_worker, target_sample_rate))
            .expect("failed to spawn asset loader thread");

        Self {
            sender: tx,
            assets,
            _worker: worker,
        }
    }

    /// Enqueue an asynchronous decode job for `id` at `path`.
    ///
    /// If the asset is already loaded or pending, this is a no-op.
    /// Returns `Err` only if the internal channel is full (capacity 256).
    pub fn load_async(
        &self,
        id: SourceId,
        path: PathBuf,
        target_sample_rate: Option<u32>,
    ) -> Result<(), MediaError> {
        {
            let mut map = self.assets.lock().expect("asset map poisoned");
            if map.contains_key(&id) {
                return Ok(()); // already loading or loaded
            }
            map.insert(id, AssetState::Pending);
        }
        self.sender
            .try_send(LoadRequest {
                id,
                path,
                target_sample_rate,
            })
            .map_err(|e| match e {
                TrySendError::Full(_) | TrySendError::Disconnected(_) => {
                    MediaError::WorkerDisconnected
                }
            })
    }

    /// Return the `Arc<DecodedAudio>` for a loaded asset, or `None` if it is
    /// not yet ready (pending, loading, or failed).
    pub fn get(&self, id: &SourceId) -> Option<Arc<DecodedAudio>> {
        let map = self.assets.lock().expect("asset map poisoned");
        match map.get(id)? {
            AssetState::Ready(arc) => Some(Arc::clone(arc)),
            _ => None,
        }
    }

    /// Return the current load status for `id`.
    pub fn status(&self, id: &SourceId) -> Option<LoadStatus> {
        let map = self.assets.lock().expect("asset map poisoned");
        Some(match map.get(id)? {
            AssetState::Pending => LoadStatus::Pending,
            AssetState::Loading => LoadStatus::Loading,
            AssetState::Ready(_) => LoadStatus::Ready,
            AssetState::Failed(e) => LoadStatus::Failed(e.clone()),
        })
    }

    /// Register an already-decoded asset under `id`.
    pub fn register(&self, id: SourceId, audio: Arc<DecodedAudio>) {
        self.assets
            .lock()
            .expect("asset map poisoned")
            .insert(id, AssetState::Ready(audio));
    }

    /// Remove an asset from the cache.  The next `load_async` for the same ID
    /// will decode it again.
    pub fn evict(&self, id: &SourceId) {
        self.assets.lock().expect("asset map poisoned").remove(id);
    }

    /// Return `true` if all queued assets have finished loading (ready or failed).
    pub fn all_ready(&self) -> bool {
        let map = self.assets.lock().expect("asset map poisoned");
        map.values()
            .all(|s| matches!(s, AssetState::Ready(_) | AssetState::Failed(_)))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Worker loop
// ─────────────────────────────────────────────────────────────────────────────

fn worker_loop(
    rx: Receiver<LoadRequest>,
    assets: Arc<Mutex<HashMap<SourceId, AssetState>>>,
    default_target_sr: Option<u32>,
) {
    // Process jobs until the sender half is dropped.
    while let Ok(req) = rx.recv() {
        // Mark as loading.
        assets
            .lock()
            .expect("asset map poisoned")
            .insert(req.id, AssetState::Loading);

        let target_sr = req.target_sample_rate.or(default_target_sr);
        let result = decode_audio(&req.path, target_sr);

        let new_state = match result {
            Ok(decoded) => AssetState::Ready(Arc::new(decoded)),
            Err(e) => AssetState::Failed(e.to_string()),
        };

        assets
            .lock()
            .expect("asset map poisoned")
            .insert(req.id, new_state);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{Duration, Instant};

    fn write_minimal_wav(path: &std::path::Path, frames: u32) {
        let channels: u16 = 2;
        let sample_rate: u32 = 44_100;
        let num_samples = (frames * channels as u32) as usize;
        let mut data: Vec<u8> = Vec::with_capacity(num_samples * 2);
        for i in 0..num_samples {
            let v = (0.3
                * (2.0 * std::f32::consts::PI * 440.0 * (i as f32 / sample_rate as f32)).sin()
                * i16::MAX as f32) as i16;
            data.extend_from_slice(&v.to_le_bytes());
        }
        let data_len = data.len() as u32;
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(b"RIFF").unwrap();
        f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
        f.write_all(b"WAVE").unwrap();
        f.write_all(b"fmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
        f.write_all(&channels.to_le_bytes()).unwrap();
        f.write_all(&sample_rate.to_le_bytes()).unwrap();
        f.write_all(&(sample_rate * channels as u32 * 2).to_le_bytes())
            .unwrap();
        f.write_all(&(channels * 2).to_le_bytes()).unwrap();
        f.write_all(&16u16.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&data_len.to_le_bytes()).unwrap();
        f.write_all(&data).unwrap();
    }

    fn make_source_id(n: u64) -> SourceId {
        esperia_project_model::audio_source::SourceId(n)
    }

    #[test]
    fn load_async_then_get() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asset.wav");
        write_minimal_wav(&path, 4410);

        let mgr = AssetManager::new(None);
        let id = make_source_id(1);
        mgr.load_async(id, path, None).unwrap();

        // Poll until ready (max 5 s).
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if Instant::now() > deadline {
                panic!("asset did not load within 5 s");
            }
            if let Some(LoadStatus::Ready) = mgr.status(&id) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        let audio = mgr.get(&id).expect("should be ready");
        assert_eq!(audio.channels, 2);
        assert_eq!(audio.sample_rate, 44_100);
        assert_eq!(audio.duration_frames, 4410);
    }

    #[test]
    fn duplicate_load_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dup.wav");
        write_minimal_wav(&path, 480);

        let mgr = AssetManager::new(None);
        let id = make_source_id(2);
        mgr.load_async(id, path.clone(), None).unwrap();
        // Second call should return Ok without panicking.
        mgr.load_async(id, path, None).unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if Instant::now() > deadline {
                panic!("timed out");
            }
            if matches!(mgr.status(&id), Some(LoadStatus::Ready)) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn failed_asset_reports_error() {
        let mgr = AssetManager::new(None);
        let id = make_source_id(3);
        mgr.load_async(id, PathBuf::from("/nonexistent/path/to/file.wav"), None)
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if Instant::now() > deadline {
                panic!("timed out waiting for failure state");
            }
            match mgr.status(&id) {
                Some(LoadStatus::Failed(_)) => break,
                Some(LoadStatus::Ready) => panic!("should not succeed"),
                _ => {}
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
