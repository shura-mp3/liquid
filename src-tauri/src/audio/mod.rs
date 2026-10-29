//! Audio capture and analysis.
//!
//! WASAPI loopback capture on a dedicated thread, feeding a lock-free ring, feeding
//! spectral analysis, feeding the visualisers. The capture thread never allocates or
//! locks in its hot path, and the DSP never touches a window handle.
//!
//! ## What this can and cannot hear
//!
//! Loopback capture takes the **mixed output of the default render endpoint**. That
//! is the whole system: the music, plus a notification, plus a game, plus a voice
//! call. Windows 10 build 19045 has no per-process loopback — that arrived with
//! build 20348 — so the signal genuinely cannot be attributed to the application
//! LIQUID is displaying metadata for. The UI says so rather than implying the
//! visualiser is watching one app.

use serde::{Deserialize, Serialize};

pub mod dsp;
pub mod ring;

#[cfg(windows)]
pub mod wasapi;

/// What the visualisers receive, quantised for the wire.
///
/// Bands and waveform are sent as bytes rather than floats: a 64-band frame plus a
/// 128-point waveform is ~200 numbers, and at 50 frames a second to three windows
/// the JSON cost of full floats is real. A byte is finer than a display pixel for
/// both, so nothing visible is lost.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioFrame {
    /// 0..255 per band.
    pub bands: Vec<u8>,
    /// -128..127, centred on zero.
    pub waveform: Vec<i8>,
    pub rms: f32,
    pub peak: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub beat: bool,
    pub silent: bool,
    /// Positive spectral change since the previous frame. Exposed because it is
    /// what beat detection runs on, and it drives the ripple strength in Phase 5.
    pub flux: f32,
    /// Monotonic ms when this frame was produced, so the UI can measure staleness.
    pub at_ms: f64,
}

impl AudioFrame {
    pub fn from_features(features: &dsp::AudioFeatures, at_ms: f64) -> Self {
        Self {
            bands: features
                .bands
                .iter()
                .map(|b| (b.clamp(0.0, 1.0) * 255.0).round() as u8)
                .collect(),
            waveform: features
                .waveform
                .iter()
                .map(|s| (s.clamp(-1.0, 1.0) * 127.0).round() as i8)
                .collect(),
            rms: features.rms,
            peak: features.peak,
            bass: features.bass,
            mid: features.mid,
            treble: features.treble,
            beat: features.beat,
            silent: features.silent,
            flux: features.spectral_flux,
            at_ms,
        }
    }

    /// A frame of nothing, for when capture is not running.
    pub fn silence(band_count: usize, waveform_points: usize, at_ms: f64) -> Self {
        Self {
            bands: vec![0; band_count],
            waveform: vec![0; waveform_points],
            rms: 0.0,
            peak: 0.0,
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            beat: false,
            silent: true,
            flux: 0.0,
            at_ms,
        }
    }
}

/// An audio endpoint the user can pick.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureState {
    Stopped,
    Running,
    /// The device went away or changed format; re-initialising.
    Recovering,
    Failed,
}

/// Capture health, for the diagnostics panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub state: CaptureState,
    pub device_name: Option<String>,
    pub sample_rate: u32,
    pub channels: u16,
    /// Samples lost to ring overruns since start. Non-zero means the DSP thread is
    /// not keeping up, which is worth seeing rather than hiding.
    pub dropped_samples: usize,
    /// Rough capture-to-feature latency: the analysis window plus the device period.
    pub latency_ms: f32,
    pub error: Option<String>,
    /// Whether this build could isolate one application's audio. On Windows 10
    /// 19045 it cannot, and saying so is better than implying otherwise.
    pub per_process_capture: bool,
}

impl Default for CaptureStatus {
    fn default() -> Self {
        Self {
            state: CaptureState::Stopped,
            device_name: None,
            sample_rate: 0,
            channels: 0,
            dropped_samples: 0,
            latency_ms: 0.0,
            error: None,
            per_process_capture: false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("audio capture is unavailable: {0}")]
    Unavailable(String),
    #[error("audio device error: {0}")]
    Device(String),
}

pub type Result<T> = std::result::Result<T, AudioError>;

#[cfg(test)]
mod tests {
    use super::*;

    fn features(bands: Vec<f32>, waveform: Vec<f32>) -> dsp::AudioFeatures {
        dsp::AudioFeatures {
            bands,
            waveform,
            ..Default::default()
        }
    }

    #[test]
    fn quantises_bands_across_the_full_byte_range() {
        let f = features(vec![0.0, 0.5, 1.0], vec![]);
        let frame = AudioFrame::from_features(&f, 0.0);
        assert_eq!(frame.bands, vec![0, 128, 255]);
    }

    #[test]
    fn clamps_out_of_range_bands_instead_of_wrapping() {
        // A u8 cast of an out-of-range float would wrap and make a loud band read
        // as silence, which is exactly the kind of glitch that looks like a bug.
        let f = features(vec![-1.0, 2.0], vec![]);
        let frame = AudioFrame::from_features(&f, 0.0);
        assert_eq!(frame.bands, vec![0, 255]);
    }

    #[test]
    fn quantises_waveform_symmetrically_about_zero() {
        let f = features(vec![], vec![-1.0, 0.0, 1.0]);
        let frame = AudioFrame::from_features(&f, 0.0);
        assert_eq!(frame.waveform, vec![-127, 0, 127]);
    }

    #[test]
    fn clamps_out_of_range_waveform_samples() {
        let f = features(vec![], vec![-4.0, 4.0]);
        let frame = AudioFrame::from_features(&f, 0.0);
        assert_eq!(frame.waveform, vec![-127, 127]);
    }

    #[test]
    fn quantisation_error_is_below_a_display_pixel() {
        // The point of sending bytes: the rounding must be finer than anything the
        // user can see on a bar a few hundred pixels tall.
        for i in 0..=1000 {
            let v = i as f32 / 1000.0;
            let frame = AudioFrame::from_features(&features(vec![v], vec![]), 0.0);
            let round_trip = f32::from(frame.bands[0]) / 255.0;
            assert!((round_trip - v).abs() <= 1.0 / 255.0);
        }
    }

    #[test]
    fn a_silence_frame_is_flat_and_marked_silent() {
        let frame = AudioFrame::silence(64, 128, 123.0);
        assert_eq!(frame.bands.len(), 64);
        assert_eq!(frame.waveform.len(), 128);
        assert!(frame.bands.iter().all(|&b| b == 0));
        assert!(frame.waveform.iter().all(|&s| s == 0));
        assert!(frame.silent);
        assert!(!frame.beat);
        assert_eq!(frame.at_ms, 123.0);
    }

    #[test]
    fn default_status_does_not_claim_per_process_capture() {
        // Windows 10 19045 cannot do it, and claiming otherwise would misrepresent
        // what the visualiser is actually watching.
        assert!(!CaptureStatus::default().per_process_capture);
        assert_eq!(CaptureStatus::default().state, CaptureState::Stopped);
    }
}
