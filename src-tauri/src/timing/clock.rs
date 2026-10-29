//! Monotonic clock backed by `QueryPerformanceCounter`.
//!
//! Every playback timestamp in LIQUID is taken from this clock, never from wall
//! time. Wall time jumps — NTP corrections, DST, the user changing the clock, resume
//! from sleep — and a jump would be integrated as playback progress, desynchronising
//! lyrics for the rest of the track.
//!
//! `QueryPerformanceCounter` is the documented high-resolution monotonic source on
//! Windows and is guaranteed to be consistent across cores on anything LIQUID
//! supports. `std::time::Instant` is also monotonic and is used as the fallback on
//! non-Windows builds so the crate still compiles for tests.

use std::time::Instant;

#[cfg(windows)]
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

/// A monotonic time source, zeroed at construction.
#[derive(Debug)]
pub struct MonotonicClock {
    #[cfg(windows)]
    frequency: i64,
    #[cfg(windows)]
    origin: i64,
    fallback_origin: Instant,
    /// True when QPC was unavailable and `Instant` is being used instead.
    degraded: bool,
}

impl MonotonicClock {
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            let mut frequency: i64 = 0;
            let mut origin: i64 = 0;
            let ok = unsafe {
                QueryPerformanceFrequency(&mut frequency).is_ok()
                    && QueryPerformanceCounter(&mut origin).is_ok()
            };
            // A zero frequency would divide by zero later, so it counts as failure.
            let usable = ok && frequency > 0;
            Self {
                frequency: if usable { frequency } else { 1 },
                origin,
                fallback_origin: Instant::now(),
                degraded: !usable,
            }
        }

        #[cfg(not(windows))]
        {
            Self {
                fallback_origin: Instant::now(),
                degraded: true,
            }
        }
    }

    /// Milliseconds since this clock was created, as a float so sub-millisecond
    /// resolution survives — truncating here would add up to 1 ms of jitter to every
    /// observation, which matters at word-level lyric timing.
    pub fn now_ms(&self) -> f64 {
        #[cfg(windows)]
        if !self.degraded {
            let mut counter: i64 = 0;
            if unsafe { QueryPerformanceCounter(&mut counter) }.is_ok() {
                let ticks = counter.saturating_sub(self.origin);
                return (ticks as f64) * 1000.0 / (self.frequency as f64);
            }
        }

        self.fallback_origin.elapsed().as_secs_f64() * 1000.0
    }

    /// True when the high-resolution counter was unavailable. Surfaced in
    /// diagnostics: it means timing accuracy is worse than normal, and the user
    /// deserves to know rather than wonder why sync is poor.
    pub fn is_degraded(&self) -> bool {
        self.degraded
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_near_zero() {
        let clock = MonotonicClock::new();
        assert!(clock.now_ms() < 50.0);
    }

    #[test]
    fn never_moves_backwards() {
        let clock = MonotonicClock::new();
        let mut last = clock.now_ms();
        for _ in 0..10_000 {
            let now = clock.now_ms();
            assert!(now >= last, "clock went backwards: {now} < {last}");
            last = now;
        }
    }

    #[test]
    fn advances_roughly_in_line_with_sleeping() {
        let clock = MonotonicClock::new();
        let before = clock.now_ms();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let elapsed = clock.now_ms() - before;
        // Generous bounds: this asserts the clock is running at the right scale,
        // not that the OS scheduler is precise.
        assert!(elapsed >= 45.0, "only advanced {elapsed} ms");
        assert!(elapsed < 500.0, "advanced an implausible {elapsed} ms");
    }

    #[test]
    fn has_sub_millisecond_resolution() {
        let clock = MonotonicClock::new();
        // Collect distinct readings within a single millisecond; a clock truncated
        // to whole milliseconds could not produce a fractional value.
        let any_fractional = (0..1000).map(|_| clock.now_ms()).any(|t| t.fract() > 0.0);
        assert!(any_fractional, "clock appears to lack sub-ms resolution");
    }
}
