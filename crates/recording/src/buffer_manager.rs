//! Preallocated, lock-free SPSC ring buffer for real-time audio capture.
//!
//! # Real-time safety contract
//! * Storage is allocated once in `CaptureRingBuffer::new()`.
//! * `push_samples()` performs zero heap allocations and zero locks.
//! * Overruns are detected deterministically and counted via `overrun_samples`.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Lock-free Single-Producer Single-Consumer ring buffer for `f32` samples.
///
/// Capacity is rounded up to a power of two so index wrapping uses bitwise AND.
pub struct CaptureRingBuffer {
    buffer: Box<[UnsafeCell<f32>]>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
    overrun_samples: AtomicU64,
}

// SAFETY: SPSC contract — `push_samples` is called only by the producer
// (audio input callback) and `pop_samples` only by the consumer (worker thread).
unsafe impl Send for CaptureRingBuffer {}
unsafe impl Sync for CaptureRingBuffer {}

impl CaptureRingBuffer {
    /// Allocate a ring buffer holding at least `min_capacity_samples` `f32` samples.
    /// Rounded up to the next power of two (minimum 64).
    pub fn new(min_capacity_samples: usize) -> Self {
        let cap = min_capacity_samples.max(64).next_power_of_two();
        let buffer: Vec<UnsafeCell<f32>> = (0..cap).map(|_| UnsafeCell::new(0.0)).collect();
        Self {
            buffer: buffer.into_boxed_slice(),
            mask: cap - 1,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            overrun_samples: AtomicU64::new(0),
        }
    }

    /// Total sample capacity of the ring buffer.
    pub fn capacity(&self) -> usize {
        self.mask + 1
    }

    /// Number of samples currently available to read.
    pub fn available_read(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        head.wrapping_sub(tail)
    }

    /// Total number of samples dropped due to buffer overruns.
    pub fn overrun_samples(&self) -> u64 {
        self.overrun_samples.load(Ordering::Acquire)
    }

    /// Reset head, tail, and overrun counter when idle.
    pub fn clear(&self) {
        let head = self.head.load(Ordering::Acquire);
        self.tail.store(head, Ordering::Release);
        self.overrun_samples.store(0, Ordering::Release);
    }

    /// Push `samples` into the ring buffer from the real-time input callback.
    ///
    /// Returns `Ok(written)` if all samples fit, or `Err(dropped)` if the buffer
    /// did not have enough free space (in which case as many samples as fit are
    /// written and `overrun_samples` is incremented by `dropped`).
    #[inline]
    pub fn push_samples(&self, samples: &[f32]) -> Result<usize, usize> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let used = head.wrapping_sub(tail);
        let free = self.capacity().saturating_sub(used);

        let to_write = samples.len().min(free);
        for (i, &sample) in samples.iter().take(to_write).enumerate() {
            let idx = (head.wrapping_add(i)) & self.mask;
            // SAFETY: SPSC producer owns slots in [head, head + free).
            unsafe {
                *self.buffer[idx].get() = sample;
            }
        }
        self.head
            .store(head.wrapping_add(to_write), Ordering::Release);

        let dropped = samples.len() - to_write;
        if dropped > 0 {
            self.overrun_samples
                .fetch_add(dropped as u64, Ordering::Relaxed);
            Err(dropped)
        } else {
            Ok(to_write)
        }
    }

    /// Pop up to `dst.len()` samples from the ring buffer on the worker thread.
    /// Returns the number of samples actually read.
    pub fn pop_samples(&self, dst: &mut [f32]) -> usize {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        let available = head.wrapping_sub(tail);
        let to_read = dst.len().min(available);

        for (i, slot) in dst.iter_mut().take(to_read).enumerate() {
            let idx = (tail.wrapping_add(i)) & self.mask;
            // SAFETY: SPSC consumer owns slots in [tail, tail + available).
            *slot = unsafe { *self.buffer[idx].get() };
        }
        self.tail
            .store(tail.wrapping_add(to_read), Ordering::Release);
        to_read
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_pop_roundtrip() {
        let ring = CaptureRingBuffer::new(64);
        let input = [0.1f32, 0.2, 0.3, 0.4];
        assert_eq!(ring.push_samples(&input), Ok(4));
        assert_eq!(ring.available_read(), 4);

        let mut out = [0.0f32; 8];
        let read = ring.pop_samples(&mut out);
        assert_eq!(read, 4);
        assert_eq!(&out[..4], &input);
        assert_eq!(ring.available_read(), 0);
    }

    #[test]
    fn detects_overrun_without_panicking() {
        let ring = CaptureRingBuffer::new(64);
        let chunk = [0.5f32; 48];
        assert_eq!(ring.push_samples(&chunk), Ok(48));
        // Only 16 slots remain; pushing 32 must drop 16.
        let overflow = [0.25f32; 32];
        assert_eq!(ring.push_samples(&overflow), Err(16));
        assert_eq!(ring.overrun_samples(), 16);
    }
}
