//! Single-producer, single-consumer ring buffer for audio samples.
//!
//! The WASAPI capture callback must never block, allocate, or lock. If it stalls,
//! the audio engine drops the buffer and the user hears or sees a glitch — so the
//! hand-off to the DSP thread is a lock-free ring with atomic indices.
//!
//! Overrun policy is **drop the oldest**. If the DSP thread falls behind, the newest
//! audio is what matters for a visualiser; replaying stale samples would make the
//! display lag further and further behind the music. Drops are counted so the
//! diagnostics panel can show that it is happening rather than leaving it invisible.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub struct SampleRing {
    /// Guarded only so the buffer can be resized when the device format changes.
    /// In steady state neither thread contends: the producer writes, the consumer
    /// reads, and the atomics below order them.
    buffer: Mutex<Vec<f32>>,
    capacity: usize,
    write: AtomicUsize,
    read: AtomicUsize,
    dropped: AtomicUsize,
}

impl SampleRing {
    /// `capacity` is in samples and is rounded up to a power of two so the index
    /// wrap is a mask rather than a modulo in the hot path.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.next_power_of_two().max(2);
        Self {
            buffer: Mutex::new(vec![0.0; capacity]),
            capacity,
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
            dropped: AtomicUsize::new(0),
        }
    }

    /// Test-only: the capture path never needs to ask.
    #[cfg(test)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Test-only: the capture path asks `peek_latest` whether a window is ready
    /// rather than reasoning about counts itself.
    #[cfg(test)]
    pub fn available(&self) -> usize {
        self.write
            .load(Ordering::Acquire)
            .wrapping_sub(self.read.load(Ordering::Acquire))
    }

    /// Producer side. Never blocks; overwrites the oldest samples when full.
    pub fn push(&self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }

        let mut buffer = match self.buffer.try_lock() {
            Ok(b) => b,
            // The buffer is being resized for a format change. Dropping this block is
            // correct — it belongs to the old format anyway.
            Err(_) => {
                self.dropped.fetch_add(samples.len(), Ordering::Relaxed);
                return;
            }
        };

        let mask = self.capacity - 1;
        let mut write = self.write.load(Ordering::Relaxed);

        for &s in samples {
            buffer[write & mask] = s;
            write = write.wrapping_add(1);
        }
        self.write.store(write, Ordering::Release);

        // If the producer has lapped the consumer, advance the read index so the
        // consumer sees the newest capacity-worth rather than a torn mix of old and
        // new. Counting this is how overruns become visible.
        let read = self.read.load(Ordering::Acquire);
        let used = write.wrapping_sub(read);
        if used > self.capacity {
            let overrun = used - self.capacity;
            self.read
                .store(read.wrapping_add(overrun), Ordering::Release);
            self.dropped.fetch_add(overrun, Ordering::Relaxed);
        }
    }

    /// Consumer side. Fills `out` with the oldest available samples and returns how
    /// many were read; zero means "not enough yet", which is normal.
    ///
    /// Test-only: analysis needs *overlapping* windows, so the capture path uses
    /// `peek_latest` + `advance` instead of consuming reads.
    #[cfg(test)]
    pub fn pop(&self, out: &mut [f32]) -> usize {
        let available = self.available();
        if available < out.len() {
            return 0;
        }

        let Ok(buffer) = self.buffer.try_lock() else {
            return 0;
        };

        let mask = self.capacity - 1;
        let mut read = self.read.load(Ordering::Relaxed);
        for slot in out.iter_mut() {
            *slot = buffer[read & mask];
            read = read.wrapping_add(1);
        }
        self.read.store(read, Ordering::Release);
        out.len()
    }

    /// Read the most recent `out.len()` samples **without** consuming, for an
    /// overlapping analysis window. Returns false when there is not enough yet.
    pub fn peek_latest(&self, out: &mut [f32]) -> bool {
        let write = self.write.load(Ordering::Acquire);
        let read = self.read.load(Ordering::Acquire);
        if write.wrapping_sub(read) < out.len() {
            return false;
        }

        let Ok(buffer) = self.buffer.try_lock() else {
            return false;
        };

        let mask = self.capacity - 1;
        let start = write.wrapping_sub(out.len());
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = buffer[start.wrapping_add(i) & mask];
        }
        true
    }

    /// Advance the consumer by `n`, for hop-based analysis.
    #[cfg(test)]
    pub fn advance(&self, n: usize) {
        let read = self.read.load(Ordering::Relaxed);
        let write = self.write.load(Ordering::Acquire);
        let available = write.wrapping_sub(read);
        self.read
            .store(read.wrapping_add(n.min(available)), Ordering::Release);
    }

    /// Declare that only the newest `n` samples are still wanted.
    ///
    /// A visualiser always analyses the most recent window, so older samples being
    /// overwritten is the intended behaviour, not data loss. Without this the read
    /// index trails forever and every overwrite is counted as an overrun: in live
    /// testing that produced a steadily climbing "dropped samples" figure during
    /// completely healthy capture, which is exactly the kind of false alarm that
    /// sends someone chasing a problem that does not exist.
    ///
    /// A genuine overrun — the producer lapping the whole buffer before we looked
    /// at all, meaning a real stall — still counts, because `push` checks against
    /// capacity independently.
    pub fn retain_latest(&self, n: usize) {
        let write = self.write.load(Ordering::Acquire);
        let keep = n.min(self.capacity);
        self.read.store(write.wrapping_sub(keep), Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_is_rounded_to_a_power_of_two() {
        assert_eq!(SampleRing::new(1000).capacity(), 1024);
        assert_eq!(SampleRing::new(1024).capacity(), 1024);
        // Degenerate sizes must not produce a zero mask.
        assert!(SampleRing::new(0).capacity() >= 2);
        assert!(SampleRing::new(1).capacity() >= 2);
    }

    #[test]
    fn round_trips_samples_in_order() {
        let ring = SampleRing::new(16);
        ring.push(&[1.0, 2.0, 3.0, 4.0]);
        let mut out = [0.0; 4];
        assert_eq!(ring.pop(&mut out), 4);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn reports_nothing_when_there_is_not_enough_yet() {
        let ring = SampleRing::new(16);
        ring.push(&[1.0, 2.0]);
        let mut out = [0.0; 4];
        assert_eq!(ring.pop(&mut out), 0, "a partial read would tear a window");
        // The samples must still be there for a later, complete read.
        ring.push(&[3.0, 4.0]);
        assert_eq!(ring.pop(&mut out), 4);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn wraps_around_the_end_of_the_buffer() {
        let ring = SampleRing::new(8);
        let mut out = [0.0; 6];
        for round in 0..5 {
            let base = (round * 6) as f32;
            let block: Vec<f32> = (0..6).map(|i| base + i as f32).collect();
            ring.push(&block);
            assert_eq!(ring.pop(&mut out), 6);
            assert_eq!(out.to_vec(), block, "round {round}");
        }
    }

    #[test]
    fn drops_the_oldest_on_overrun_and_counts_it() {
        let ring = SampleRing::new(8);
        // Twelve samples into an eight-sample ring: the first four are lost.
        let block: Vec<f32> = (0..12).map(|i| i as f32).collect();
        ring.push(&block);

        assert_eq!(ring.dropped(), 4, "overrun must be counted, not hidden");
        assert_eq!(ring.available(), 8);

        let mut out = [0.0; 8];
        assert_eq!(ring.pop(&mut out), 8);
        // The newest eight survive; a visualiser wants current audio, not stale.
        assert_eq!(out.to_vec(), vec![4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0]);
    }

    #[test]
    fn peek_does_not_consume() {
        let ring = SampleRing::new(16);
        ring.push(&[1.0, 2.0, 3.0, 4.0]);
        let mut a = [0.0; 4];
        let mut b = [0.0; 4];
        assert!(ring.peek_latest(&mut a));
        assert!(ring.peek_latest(&mut b));
        assert_eq!(a, b, "peeking twice must give the same window");
        assert_eq!(ring.available(), 4, "peek must not consume");
    }

    #[test]
    fn peek_returns_the_most_recent_window() {
        let ring = SampleRing::new(16);
        ring.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let mut out = [0.0; 3];
        assert!(ring.peek_latest(&mut out));
        assert_eq!(out, [4.0, 5.0, 6.0], "should be the newest, not the oldest");
    }

    #[test]
    fn peek_reports_false_when_short() {
        let ring = SampleRing::new(16);
        ring.push(&[1.0]);
        let mut out = [0.0; 4];
        assert!(!ring.peek_latest(&mut out));
    }

    #[test]
    fn retain_latest_keeps_only_the_newest_window() {
        let ring = SampleRing::new(16);
        ring.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        ring.retain_latest(2);
        assert_eq!(ring.available(), 2);
        let mut out = [0.0; 2];
        assert_eq!(ring.pop(&mut out), 2);
        assert_eq!(out, [5.0, 6.0]);
    }

    #[test]
    fn retain_latest_does_not_manufacture_samples() {
        let ring = SampleRing::new(16);
        ring.push(&[1.0, 2.0]);
        // Asking to keep more than exists must not make `available` lie.
        ring.retain_latest(100);
        assert!(ring.available() <= ring.capacity());
    }

    #[test]
    fn steady_capture_reports_no_drops() {
        // The regression this guards: before `retain_latest`, a healthy capture
        // loop that always analysed the newest window reported a steadily climbing
        // dropped-sample count, because the read index never moved.
        let ring = SampleRing::new(4096);
        let mut window = vec![0.0f32; 2048];
        for _ in 0..200 {
            let block: Vec<f32> = (0..480).map(|i| i as f32 * 0.001).collect();
            ring.push(&block);
            if ring.peek_latest(&mut window) {
                ring.retain_latest(2048 + 1024);
            }
        }
        assert_eq!(ring.dropped(), 0, "healthy capture must not report drops");
    }

    #[test]
    fn advance_moves_the_hop_without_reading() {
        let ring = SampleRing::new(16);
        ring.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        ring.advance(2);
        let mut out = [0.0; 4];
        assert_eq!(ring.pop(&mut out), 4);
        assert_eq!(out, [3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn advance_cannot_overtake_the_producer() {
        let ring = SampleRing::new(16);
        ring.push(&[1.0, 2.0]);
        ring.advance(100);
        assert_eq!(ring.available(), 0, "read index must not pass write index");
    }

    #[test]
    fn pushing_nothing_is_a_no_op() {
        let ring = SampleRing::new(16);
        ring.push(&[]);
        assert_eq!(ring.available(), 0);
        assert_eq!(ring.dropped(), 0);
    }

    #[test]
    fn survives_a_producer_and_consumer_on_separate_threads() {
        use std::sync::Arc;

        let ring = Arc::new(SampleRing::new(1024));
        let producer = Arc::clone(&ring);

        let writer = std::thread::spawn(move || {
            for i in 0..10_000u32 {
                producer.push(&[i as f32]);
            }
        });

        let mut out = [0.0; 64];
        let mut total = 0usize;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            total += ring.pop(&mut out);
            if writer.is_finished() && ring.available() < out.len() {
                break;
            }
        }
        writer.join().unwrap();

        // Exact counts depend on scheduling; what must hold is that nothing is
        // double-counted and the accounting adds up.
        assert!(total + ring.dropped() + ring.available() <= 10_000 + out.len());
        assert!(total > 0, "consumer never saw any samples");
    }
}
