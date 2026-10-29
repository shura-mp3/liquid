//! Spectral analysis.
//!
//! Turns a window of PCM into the features a visualiser needs. Everything here is
//! pure given its configuration, so it is tested against synthetic signals with
//! known answers — a 1 kHz sine must light the 1 kHz band and nothing else, silence
//! must produce stillness, and the loudness figures must match the arithmetic.
//!
//! Two choices worth stating:
//!
//! **Bands are logarithmic, not linear.** A linear FFT bin mapping puts almost
//! everything musical in the bottom few pixels: at 48 kHz with a 2048-point
//! transform, every note below the top octave lands in the first eighth of the
//! spectrum. Ears are logarithmic, so the bands are too.
//!
//! **Magnitudes are converted to decibels.** Linear magnitude makes quiet passages
//! look like silence and loud ones clip the display. A dB floor maps the useful
//! range onto 0..1.

use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use std::sync::Arc;

/// Analysis window. 2048 at 48 kHz is ~43 ms — long enough to resolve bass
/// (23 Hz per bin) and short enough to feel immediate.
pub const FFT_SIZE: usize = 2048;

/// 50% overlap, so features update about twice per window.
pub const HOP_SIZE: usize = FFT_SIZE / 2;

/// Lowest band edge. Below this is mostly DC offset and rumble.
const MIN_HZ: f32 = 30.0;
/// Highest band edge. Above this there is little musical energy and a lot of hiss.
const MAX_HZ: f32 = 16_000.0;

/// Anything below this maps to 0. About the noise floor of 16-bit audio.
const DB_FLOOR: f32 = -70.0;

/// Below this RMS the signal is treated as silence, so the display goes still
/// rather than animating dither noise.
const SILENCE_RMS: f32 = 0.0005;

/// Beats closer together than this are suppressed. 300 ms is 200 BPM.
const BEAT_REFRACTORY_FRAMES: usize = 7;

#[derive(Debug, Clone, Copy)]
pub struct DspConfig {
    pub sample_rate: u32,
    pub band_count: usize,
    /// How quickly a band may rise, as a fraction per frame. 1.0 is instant.
    pub attack: f32,
    /// How quickly a band may fall. Lower is smoother.
    pub release: f32,
}

impl Default for DspConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            band_count: 64,
            // Rise instantly, fall smoothly: a bar that lags the transient feels
            // broken, but one that drops instantly flickers.
            attack: 1.0,
            release: 0.18,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AudioFeatures {
    /// Per-band energy, 0..1, log-spaced and smoothed.
    pub bands: Vec<f32>,
    /// Downsampled waveform, -1..1.
    pub waveform: Vec<f32>,
    pub rms: f32,
    pub peak: f32,
    /// Energy in the lowest third of the bands, 0..1.
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    /// Positive spectral change since the previous frame.
    pub spectral_flux: f32,
    pub beat: bool,
    pub silent: bool,
}

pub struct Analyser {
    config: DspConfig,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// Bin range per band, precomputed.
    band_bins: Vec<(usize, usize)>,
    scratch: Vec<Complex32>,
    magnitudes: Vec<f32>,
    previous_magnitudes: Vec<f32>,
    smoothed: Vec<f32>,
    /// Recent flux values, for the adaptive beat threshold.
    flux_history: Vec<f32>,
    frames_since_beat: usize,
}

impl Analyser {
    pub fn new(config: DspConfig) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);

        // Hann window. Without it, a tone that does not land exactly on a bin
        // smears energy across the whole spectrum and every band lights up.
        let window = (0..FFT_SIZE)
            .map(|i| {
                let t = i as f32 / (FFT_SIZE - 1) as f32;
                0.5 - 0.5 * (std::f32::consts::TAU * t).cos()
            })
            .collect();

        let band_bins = compute_band_bins(config.band_count, config.sample_rate);
        let bins = FFT_SIZE / 2;

        Self {
            config,
            fft,
            window,
            band_bins,
            scratch: vec![Complex32::new(0.0, 0.0); FFT_SIZE],
            magnitudes: vec![0.0; bins],
            previous_magnitudes: vec![0.0; bins],
            smoothed: vec![0.0; config.band_count],
            flux_history: Vec::with_capacity(64),
            frames_since_beat: BEAT_REFRACTORY_FRAMES,
        }
    }

    /// Analyse one window. `samples` must be exactly `FFT_SIZE` long.
    pub fn analyse(&mut self, samples: &[f32], waveform_points: usize) -> AudioFeatures {
        debug_assert_eq!(samples.len(), FFT_SIZE);

        let mut sum_squares = 0.0f32;
        let mut peak = 0.0f32;
        for &s in samples {
            sum_squares += s * s;
            peak = peak.max(s.abs());
        }
        let rms = (sum_squares / samples.len() as f32).sqrt();
        let silent = rms < SILENCE_RMS;

        for (i, (&s, &w)) in samples.iter().zip(self.window.iter()).enumerate() {
            self.scratch[i] = Complex32::new(s * w, 0.0);
        }
        self.fft.process(&mut self.scratch);

        // Only the first half is meaningful for a real input; the rest mirrors it.
        std::mem::swap(&mut self.previous_magnitudes, &mut self.magnitudes);
        // Normalise so a full-scale sine reads about 1.0 regardless of FFT size.
        // The 2.0 accounts for the discarded mirror half, and 0.5 for the Hann
        // window's coherent gain.
        let scale = 2.0 / (FFT_SIZE as f32 * 0.5);
        for (i, mag) in self.magnitudes.iter_mut().enumerate() {
            *mag = self.scratch[i].norm() * scale;
        }

        // Spectral flux: only increases count. Energy decaying is not an onset.
        let flux: f32 = self
            .magnitudes
            .iter()
            .zip(self.previous_magnitudes.iter())
            .map(|(now, before)| (now - before).max(0.0))
            .sum();

        let beat = self.detect_beat(flux, silent);

        for (band, &(lo, hi)) in self.band_bins.iter().enumerate() {
            // Peak within the band, not mean: a mean across a wide high band buries
            // a real tone under the quiet bins beside it.
            let mut magnitude = 0.0f32;
            for bin in lo..hi.min(self.magnitudes.len()) {
                magnitude = magnitude.max(self.magnitudes[bin]);
            }

            let target = if silent { 0.0 } else { normalise_db(magnitude) };
            let current = self.smoothed[band];
            let rate = if target > current {
                self.config.attack
            } else {
                self.config.release
            };
            self.smoothed[band] = current + (target - current) * rate.clamp(0.0, 1.0);
        }

        let third = (self.config.band_count / 3).max(1);
        let mean = |slice: &[f32]| {
            if slice.is_empty() {
                0.0
            } else {
                slice.iter().sum::<f32>() / slice.len() as f32
            }
        };

        AudioFeatures {
            bands: self.smoothed.clone(),
            waveform: downsample(samples, waveform_points),
            rms,
            peak,
            bass: mean(&self.smoothed[..third]),
            mid: mean(&self.smoothed[third..(third * 2).min(self.smoothed.len())]),
            treble: mean(&self.smoothed[(third * 2).min(self.smoothed.len())..]),
            spectral_flux: flux,
            beat,
            silent,
        }
    }

    /// Onset detection against an adaptive threshold.
    ///
    /// A fixed threshold works for exactly one track at one volume. The running mean
    /// of recent flux plus a margin adapts to the material, and the refractory
    /// period stops a single transient firing several frames in a row.
    fn detect_beat(&mut self, flux: f32, silent: bool) -> bool {
        self.frames_since_beat = self.frames_since_beat.saturating_add(1);

        self.flux_history.push(flux);
        if self.flux_history.len() > 43 {
            self.flux_history.remove(0);
        }

        if silent || self.flux_history.len() < 8 {
            return false;
        }

        let mean = self.flux_history.iter().sum::<f32>() / self.flux_history.len() as f32;
        let variance = self
            .flux_history
            .iter()
            .map(|f| (f - mean) * (f - mean))
            .sum::<f32>()
            / self.flux_history.len() as f32;
        let threshold = mean + 1.5 * variance.sqrt();

        if flux > threshold && flux > 0.0 && self.frames_since_beat >= BEAT_REFRACTORY_FRAMES {
            self.frames_since_beat = 0;
            return true;
        }
        false
    }
}

/// Map magnitude to 0..1 through decibels.
fn normalise_db(magnitude: f32) -> f32 {
    if magnitude <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * magnitude.log10();
    ((db - DB_FLOOR) / -DB_FLOOR).clamp(0.0, 1.0)
}

/// Logarithmically spaced bin ranges, one per band.
///
/// Every band gets at least one bin: at the bottom of the range several bands would
/// otherwise map to the same bin and collapse to an empty range, leaving dead bars.
fn compute_band_bins(band_count: usize, sample_rate: u32) -> Vec<(usize, usize)> {
    let bins = FFT_SIZE / 2;
    let hz_per_bin = sample_rate as f32 / FFT_SIZE as f32;
    let max_hz = MAX_HZ.min(sample_rate as f32 / 2.0);

    let mut out = Vec::with_capacity(band_count);
    let mut previous_hi = 0usize;

    for band in 0..band_count {
        let t0 = band as f32 / band_count as f32;
        let t1 = (band + 1) as f32 / band_count as f32;
        let f0 = MIN_HZ * (max_hz / MIN_HZ).powf(t0);
        let f1 = MIN_HZ * (max_hz / MIN_HZ).powf(t1);

        let lo = ((f0 / hz_per_bin).floor() as usize).clamp(1, bins.saturating_sub(1));
        let mut hi = ((f1 / hz_per_bin).ceil() as usize).clamp(1, bins);

        let lo = lo.max(previous_hi.min(bins.saturating_sub(1)));
        if hi <= lo {
            hi = (lo + 1).min(bins);
        }
        previous_hi = hi;
        out.push((lo, hi));
    }
    out
}

/// Reduce a window to `points` samples for display, keeping the extremes.
///
/// Averaging would flatten a waveform into a thin line, because positive and
/// negative excursions cancel. Taking the largest absolute value in each chunk keeps
/// the shape recognisable.
fn downsample(samples: &[f32], points: usize) -> Vec<f32> {
    if points == 0 || samples.is_empty() {
        return Vec::new();
    }
    let chunk = samples.len().div_ceil(points);
    samples
        .chunks(chunk)
        .map(|c| {
            let mut extreme = 0.0f32;
            for &s in c {
                if s.abs() > extreme.abs() {
                    extreme = s;
                }
            }
            extreme
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn sine(hz: f32, amplitude: f32, len: usize, sample_rate: u32) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                amplitude * (std::f32::consts::TAU * hz * t).sin()
            })
            .collect()
    }

    fn analyser(bands: usize) -> Analyser {
        Analyser::new(DspConfig {
            sample_rate: SR,
            band_count: bands,
            // No smoothing, so a single frame reflects the signal exactly.
            attack: 1.0,
            release: 1.0,
        })
    }

    /// Which band covers a frequency, per the band mapping.
    fn band_of(hz: f32, bands: usize) -> usize {
        let edges = compute_band_bins(bands, SR);
        let hz_per_bin = SR as f32 / FFT_SIZE as f32;
        let bin = (hz / hz_per_bin).round() as usize;
        edges
            .iter()
            .position(|&(lo, hi)| bin >= lo && bin < hi)
            .unwrap_or(0)
    }

    // ---- band mapping -----------------------------------------------------

    #[test]
    fn band_ranges_are_ordered_and_non_empty() {
        for count in [8, 16, 32, 64, 128, 256] {
            let edges = compute_band_bins(count, SR);
            assert_eq!(edges.len(), count);
            for (i, &(lo, hi)) in edges.iter().enumerate() {
                assert!(hi > lo, "band {i} of {count} is empty: {lo}..{hi}");
                assert!(lo >= 1, "band {i} must skip the DC bin");
            }
            for pair in edges.windows(2) {
                assert!(pair[1].0 >= pair[0].0, "bands must ascend");
            }
        }
    }

    #[test]
    fn bands_are_logarithmic_so_bass_is_not_crushed() {
        let edges = compute_band_bins(32, SR);
        let first_width = edges[0].1 - edges[0].0;
        let last_width = edges[31].1 - edges[31].0;
        // The top band must span far more bins than the bottom one; that is the
        // whole point of a log mapping.
        assert!(
            last_width > first_width * 10,
            "expected a log spread, got {first_width} vs {last_width}"
        );
    }

    #[test]
    fn band_mapping_adapts_to_a_lower_sample_rate() {
        // At 8 kHz the Nyquist limit is below MAX_HZ, so the range must shrink
        // rather than index past the end of the spectrum.
        let edges = compute_band_bins(16, 8_000);
        let bins = FFT_SIZE / 2;
        for &(lo, hi) in &edges {
            assert!(hi <= bins);
            assert!(lo < bins);
        }
    }

    // ---- tones ------------------------------------------------------------

    #[test]
    fn a_1khz_tone_lights_the_1khz_band() {
        let mut a = analyser(64);
        let f = a.analyse(&sine(1000.0, 0.8, FFT_SIZE, SR), 64);

        let expected = band_of(1000.0, 64);
        let loudest = f
            .bands
            .iter()
            .enumerate()
            .max_by(|x, y| x.1.partial_cmp(y.1).unwrap())
            .unwrap()
            .0;

        assert!(
            (loudest as i32 - expected as i32).abs() <= 1,
            "expected the peak near band {expected}, got {loudest}"
        );
        assert!(
            f.bands[loudest] > 0.6,
            "tone should be strong, got {}",
            f.bands[loudest]
        );
    }

    #[test]
    fn energy_does_not_smear_across_the_whole_spectrum() {
        // This is what the Hann window buys: without it a tone between bins leaks
        // everywhere and every bar lights up.
        let mut a = analyser(64);
        let f = a.analyse(&sine(1000.0, 0.8, FFT_SIZE, SR), 64);

        let peak_band = band_of(1000.0, 64);
        let far: Vec<f32> = f
            .bands
            .iter()
            .enumerate()
            .filter(|(i, _)| (*i as i32 - peak_band as i32).abs() > 6)
            .map(|(_, v)| *v)
            .collect();

        let worst = far.iter().cloned().fold(0.0f32, f32::max);
        assert!(worst < 0.45, "leakage into distant bands too high: {worst}");
    }

    #[test]
    fn a_low_tone_and_a_high_tone_land_in_different_places() {
        let mut a = analyser(64);
        let low = a.analyse(&sine(80.0, 0.8, FFT_SIZE, SR), 32);
        let mut b = analyser(64);
        let high = b.analyse(&sine(8000.0, 0.8, FFT_SIZE, SR), 32);

        assert!(low.bass > low.treble, "80 Hz should be bass-dominant");
        assert!(high.treble > high.bass, "8 kHz should be treble-dominant");
    }

    // ---- loudness ---------------------------------------------------------

    #[test]
    fn rms_of_a_sine_matches_the_arithmetic() {
        let mut a = analyser(16);
        let amplitude = 0.5;
        let f = a.analyse(&sine(1000.0, amplitude, FFT_SIZE, SR), 16);
        // RMS of a sine is A/sqrt(2).
        let expected = amplitude / 2.0f32.sqrt();
        assert!(
            (f.rms - expected).abs() < 0.01,
            "expected rms ~{expected}, got {}",
            f.rms
        );
    }

    #[test]
    fn peak_tracks_the_largest_excursion() {
        let mut a = analyser(16);
        let f = a.analyse(&sine(1000.0, 0.73, FFT_SIZE, SR), 16);
        assert!((f.peak - 0.73).abs() < 0.02, "got {}", f.peak);
    }

    // ---- silence ----------------------------------------------------------

    #[test]
    fn silence_produces_stillness_not_noise() {
        let mut a = analyser(64);
        let f = a.analyse(&vec![0.0; FFT_SIZE], 64);

        assert!(f.silent);
        assert!(!f.beat, "silence must never register a beat");
        assert_eq!(f.rms, 0.0);
        assert!(
            f.bands.iter().all(|&b| b == 0.0),
            "every band must be flat in silence"
        );
    }

    #[test]
    fn dither_level_noise_still_counts_as_silence() {
        let mut a = analyser(32);
        // Far below the gate: this is what a muted stream actually looks like.
        let samples: Vec<f32> = (0..FFT_SIZE)
            .map(|i| if i % 2 == 0 { 1e-5 } else { -1e-5 })
            .collect();
        assert!(a.analyse(&samples, 32).silent);
    }

    #[test]
    fn a_quiet_but_real_signal_is_not_silence() {
        let mut a = analyser(32);
        let f = a.analyse(&sine(1000.0, 0.02, FFT_SIZE, SR), 32);
        assert!(!f.silent, "a real quiet passage must still animate");
    }

    // ---- smoothing --------------------------------------------------------

    #[test]
    fn release_smoothing_makes_bands_fall_gradually() {
        let mut a = Analyser::new(DspConfig {
            sample_rate: SR,
            band_count: 32,
            attack: 1.0,
            release: 0.2,
        });

        let loud = a.analyse(&sine(1000.0, 0.9, FFT_SIZE, SR), 16);
        let band = band_of(1000.0, 32);
        let high = loud.bands[band];
        assert!(high > 0.5);

        // Now silence: the band must decay, not snap to zero.
        let after = a.analyse(&vec![0.0; FFT_SIZE], 16);
        assert!(after.bands[band] < high, "band should fall");
        assert!(
            after.bands[band] > 0.0,
            "release smoothing should leave a tail, got {}",
            after.bands[band]
        );
    }

    #[test]
    fn attack_lets_a_transient_through_immediately() {
        let mut a = Analyser::new(DspConfig {
            sample_rate: SR,
            band_count: 32,
            attack: 1.0,
            release: 0.1,
        });
        a.analyse(&vec![0.0; FFT_SIZE], 16);
        let hit = a.analyse(&sine(1000.0, 0.9, FFT_SIZE, SR), 16);
        assert!(
            hit.bands[band_of(1000.0, 32)] > 0.5,
            "a transient must not be smoothed away"
        );
    }

    // ---- onsets -----------------------------------------------------------

    #[test]
    fn a_sudden_onset_registers_a_beat() {
        let mut a = analyser(32);
        // Settle the adaptive threshold on quiet material first.
        for _ in 0..12 {
            a.analyse(&sine(1000.0, 0.02, FFT_SIZE, SR), 16);
        }
        let mut fired = false;
        for _ in 0..4 {
            if a.analyse(&sine(1000.0, 0.9, FFT_SIZE, SR), 16).beat {
                fired = true;
                break;
            }
        }
        assert!(fired, "a large jump in energy should register as an onset");
    }

    #[test]
    fn beats_respect_a_refractory_period() {
        let mut a = analyser(32);
        let loud = sine(1000.0, 0.9, FFT_SIZE, SR);
        for _ in 0..12 {
            a.analyse(&sine(1000.0, 0.02, FFT_SIZE, SR), 16);
        }
        let mut beats = 0;
        for _ in 0..BEAT_REFRACTORY_FRAMES {
            if a.analyse(&loud, 16).beat {
                beats += 1;
            }
        }
        assert!(
            beats <= 1,
            "one transient must not fire repeatedly, got {beats}"
        );
    }

    #[test]
    fn steady_tone_does_not_fire_continuous_beats() {
        let mut a = analyser(32);
        let tone = sine(440.0, 0.5, FFT_SIZE, SR);
        let mut beats = 0;
        for _ in 0..40 {
            if a.analyse(&tone, 16).beat {
                beats += 1;
            }
        }
        assert!(
            beats <= 2,
            "a constant tone is not a drum track, got {beats}"
        );
    }

    // ---- waveform ---------------------------------------------------------

    #[test]
    fn waveform_keeps_its_shape_rather_than_averaging_to_zero() {
        let mut a = analyser(16);
        let f = a.analyse(&sine(200.0, 0.9, FFT_SIZE, SR), 128);
        assert_eq!(f.waveform.len(), 128);

        let max = f.waveform.iter().cloned().fold(f32::MIN, f32::max);
        let min = f.waveform.iter().cloned().fold(f32::MAX, f32::min);
        // Averaging would collapse this towards zero on both sides.
        assert!(max > 0.5, "lost the positive excursion: {max}");
        assert!(min < -0.5, "lost the negative excursion: {min}");
    }

    #[test]
    fn waveform_handles_degenerate_requests() {
        let mut a = analyser(16);
        assert!(a.analyse(&vec![0.0; FFT_SIZE], 0).waveform.is_empty());
    }

    // ---- normalisation ----------------------------------------------------

    #[test]
    fn db_normalisation_covers_the_expected_range() {
        assert_eq!(normalise_db(0.0), 0.0);
        assert_eq!(
            normalise_db(-1.0),
            0.0,
            "negative magnitude is not possible"
        );
        assert!(
            (normalise_db(1.0) - 1.0).abs() < 1e-6,
            "full scale maps to 1"
        );
        // Below the floor clamps to zero rather than going negative.
        assert_eq!(normalise_db(1e-9), 0.0);
        let mid = normalise_db(0.01); // -40 dB
        assert!(mid > 0.0 && mid < 1.0, "got {mid}");
    }

    #[test]
    fn bands_always_stay_within_zero_and_one() {
        let mut a = analyser(64);
        // Deliberately over-driven input.
        let hot: Vec<f32> = sine(1000.0, 4.0, FFT_SIZE, SR);
        let f = a.analyse(&hot, 32);
        for (i, &b) in f.bands.iter().enumerate() {
            assert!((0.0..=1.0).contains(&b), "band {i} out of range: {b}");
        }
    }
}
