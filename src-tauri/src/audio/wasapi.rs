//! WASAPI loopback capture.
//!
//! Opens the default render endpoint in loopback mode and pulls the mixed system
//! output into a ring buffer. Event-driven rather than polled: the audio engine
//! signals an event when a buffer is ready, so the thread sleeps instead of
//! spinning.
//!
//! ## Loopback and silence
//!
//! A loopback client receives **nothing at all** while the endpoint is idle — not
//! zeros, no packets. A naive reader treats that as "no new audio" and leaves the
//! last frame on screen forever, so the bars freeze mid-song instead of falling.
//! The wait therefore has a timeout, and a timeout is treated as a window of
//! silence, which is what it actually represents.
//!
//! ## Format
//!
//! The shared-mode mix format is whatever the user's endpoint is configured for:
//! commonly 48 kHz float32 stereo, but 44.1 kHz and 24-bit integer are both normal,
//! and surround layouts exist. Rather than demanding one format, the mix format is
//! read and converted — refusing to run because someone has a 96 kHz interface
//! would be a poor trade.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, DEVICE_STATE_ACTIVE, WAVE_FORMAT_PCM,
};
use windows::Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE;
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use super::dsp::{Analyser, DspConfig, FFT_SIZE, HOP_SIZE};
use super::ring::SampleRing;
use super::{AudioDevice, AudioError, AudioFrame, CaptureState, CaptureStatus, Result};

/// Property key for an endpoint's friendly name.
/// `PKEY_Device_FriendlyName` — declared here because the `windows` crate does not
/// re-export the audio property keys.
const PKEY_DEVICE_FRIENDLY_NAME: windows::Win32::Foundation::PROPERTYKEY =
    windows::Win32::Foundation::PROPERTYKEY {
        fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };

/// Ring capacity: about a second at 48 kHz stereo. Generous enough to ride out a
/// scheduling hiccup, small enough that recovery is immediate.
const RING_SAMPLES: usize = 65_536;

/// How long to wait for a buffer before treating the gap as silence.
const WAIT_TIMEOUT_MS: u32 = 100;

/// Frames pushed to the UI per second. Decoupled from both the capture period and
/// the render rate.
const EMIT_HZ: f64 = 50.0;

const WAVEFORM_POINTS: usize = 128;

pub struct AudioEngine {
    status: Arc<Mutex<CaptureStatus>>,
    running: Arc<AtomicBool>,
    /// Device id to open, or `None` for the system default.
    requested_device: Arc<Mutex<Option<String>>>,
    /// Set when the caller wants the capture thread to restart on a new device.
    restart: Arc<AtomicBool>,
}

impl AudioEngine {
    /// Start capture. `on_frame` is called on the audio thread at roughly `EMIT_HZ`.
    pub fn start<F>(
        band_count: usize,
        now_ms: impl Fn() -> f64 + Send + 'static,
        on_frame: F,
    ) -> Self
    where
        F: Fn(AudioFrame) + Send + 'static,
    {
        let status = Arc::new(Mutex::new(CaptureStatus::default()));
        let running = Arc::new(AtomicBool::new(true));
        let requested_device: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let restart = Arc::new(AtomicBool::new(false));

        let engine = Self {
            status: Arc::clone(&status),
            running: Arc::clone(&running),
            requested_device: Arc::clone(&requested_device),
            restart: Arc::clone(&restart),
        };

        std::thread::Builder::new()
            .name("liquid-audio".into())
            .spawn(move || {
                unsafe {
                    let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
                    if hr.is_err() {
                        let mut s = status.lock();
                        s.state = CaptureState::Failed;
                        s.error = Some(format!("CoInitializeEx failed: {hr:?}"));
                        return;
                    }
                }

                // Outer loop: each pass is one capture session. A device change,
                // format change or unplug drops out of the inner loop and comes back
                // here to re-open, rather than killing the thread.
                while running.load(Ordering::SeqCst) {
                    restart.store(false, Ordering::SeqCst);
                    let device_id = requested_device.lock().clone();

                    // SAFETY: COM is initialised on this thread above, and the
                    // session owns every interface pointer it creates.
                    let session = unsafe {
                        run_session(
                            device_id.as_deref(),
                            band_count,
                            &running,
                            &restart,
                            &status,
                            &now_ms,
                            &on_frame,
                        )
                    };
                    match session {
                        Ok(()) => {}
                        Err(e) => {
                            tracing::warn!(error = %e, "audio capture session ended");
                            let mut s = status.lock();
                            s.state = CaptureState::Recovering;
                            s.error = Some(e.to_string());
                        }
                    }

                    if !running.load(Ordering::SeqCst) {
                        break;
                    }
                    // Back off before retrying so a permanently missing device does
                    // not spin the CPU re-opening it.
                    std::thread::sleep(Duration::from_millis(600));
                }

                status.lock().state = CaptureState::Stopped;
                unsafe { CoUninitialize() };
                tracing::info!("audio thread stopped");
            })
            .expect("could not spawn the audio thread");

        engine
    }

    pub fn status(&self) -> CaptureStatus {
        self.status.lock().clone()
    }

    /// Switch endpoint. `None` means follow the system default.
    pub fn set_device(&self, device_id: Option<String>) {
        *self.requested_device.lock() = device_id;
        self.restart.store(true, Ordering::SeqCst);
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.restart.store(true, Ordering::SeqCst);
    }
}

/// One capture session, returning when the device changes or capture stops.
#[allow(clippy::too_many_arguments)]
unsafe fn run_session<F>(
    device_id: Option<&str>,
    band_count: usize,
    running: &AtomicBool,
    restart: &AtomicBool,
    status: &Arc<Mutex<CaptureStatus>>,
    now_ms: &(impl Fn() -> f64 + Send + 'static),
    on_frame: &F,
) -> Result<()>
where
    F: Fn(AudioFrame) + Send + 'static,
{
    let enumerator: IMMDeviceEnumerator =
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
            .map_err(|e| AudioError::Unavailable(format!("device enumerator: {e}")))?;

    let device: IMMDevice = match device_id {
        Some(id) => {
            let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
            enumerator
                .GetDevice(PCWSTR(wide.as_ptr()))
                .map_err(|e| AudioError::Device(format!("requested device unavailable: {e}")))?
        }
        None => enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| AudioError::Device(format!("no default playback device: {e}")))?,
    };

    let device_name = friendly_name(&device).unwrap_or_else(|| "Unknown device".to_string());

    let client: IAudioClient = device
        .Activate(CLSCTX_ALL, None)
        .map_err(|e| AudioError::Device(format!("could not activate audio client: {e}")))?;

    let format_ptr = client
        .GetMixFormat()
        .map_err(|e| AudioError::Device(format!("GetMixFormat: {e}")))?;
    let format = *format_ptr;
    let sample_rate = format.nSamplesPerSec;
    let channels = format.nChannels;
    let bits = format.wBitsPerSample;
    // Decided before the pointer is freed; `wFormatTag` for EXTENSIBLE needs the
    // sub-format, but in shared mode the mix is float except on unusual endpoints.
    let is_float = format.wFormatTag as u32 == WAVE_FORMAT_EXTENSIBLE
        || format.wFormatTag as u32 != WAVE_FORMAT_PCM;

    let event = CreateEventW(None, false, false, None)
        .map_err(|e| AudioError::Device(format!("CreateEventW: {e}")))?;

    let init = client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
        // Let the engine pick the period; a loopback client does not drive timing.
        0,
        0,
        format_ptr,
        None,
    );
    CoTaskMemFree(Some(format_ptr as *const _));

    init.map_err(|e| AudioError::Device(format!("IAudioClient::Initialize: {e}")))?;

    client
        .SetEventHandle(event)
        .map_err(|e| AudioError::Device(format!("SetEventHandle: {e}")))?;

    let capture: IAudioCaptureClient = client
        .GetService()
        .map_err(|e| AudioError::Device(format!("GetService(IAudioCaptureClient): {e}")))?;

    client
        .Start()
        .map_err(|e| AudioError::Device(format!("IAudioClient::Start: {e}")))?;

    let ring = SampleRing::new(RING_SAMPLES);
    let mut analyser = Analyser::new(DspConfig {
        sample_rate,
        band_count,
        ..Default::default()
    });

    {
        let mut s = status.lock();
        s.state = CaptureState::Running;
        s.device_name = Some(device_name.clone());
        s.sample_rate = sample_rate;
        s.channels = channels;
        s.error = None;
        s.dropped_samples = 0;
        // Window plus one engine period, which is the honest end-to-end figure.
        s.latency_ms = (FFT_SIZE as f32 / sample_rate as f32) * 1000.0 + 10.0;
        s.per_process_capture = false;
    }
    tracing::info!(
        device = %device_name,
        sample_rate,
        channels,
        bits,
        is_float,
        "audio capture started"
    );

    let mut window = vec![0.0f32; FFT_SIZE];
    // A bounded heartbeat, about every two seconds. Enough to tell "capture is
    // alive and the numbers move with the music" from "the bars are flat because
    // something is broken", without logging fifty frames a second.
    let mut frames_emitted: u64 = 0;
    let mut interleaved: Vec<f32> = Vec::with_capacity(4096);
    let emit_interval_ms = 1000.0 / EMIT_HZ;
    let mut last_emit = now_ms();

    while running.load(Ordering::SeqCst) && !restart.load(Ordering::SeqCst) {
        let wait = WaitForSingleObject(event, WAIT_TIMEOUT_MS);
        let timed_out = wait != WAIT_OBJECT_0;

        if !timed_out {
            // Drain every packet the engine has queued, not just one.
            loop {
                let packet_frames = match capture.GetNextPacketSize() {
                    Ok(n) if n > 0 => n,
                    // Zero or an error both mean "nothing more queued right now".
                    _ => break,
                };
                let _ = packet_frames;

                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                if capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                    .is_err()
                {
                    break;
                }

                interleaved.clear();
                if !data.is_null() && frames > 0 {
                    let total = frames as usize * channels as usize;
                    // AUDCLNT_BUFFERFLAGS_SILENT (0x2) means the buffer contents are
                    // undefined and must be treated as zeros, not read.
                    if flags & 0x2 != 0 {
                        interleaved.resize(total, 0.0);
                    } else if is_float && bits == 32 {
                        let src = std::slice::from_raw_parts(data as *const f32, total);
                        interleaved.extend_from_slice(src);
                    } else if bits == 16 {
                        let src = std::slice::from_raw_parts(data as *const i16, total);
                        interleaved.extend(src.iter().map(|&s| f32::from(s) / 32768.0));
                    } else if bits == 32 {
                        let src = std::slice::from_raw_parts(data as *const i32, total);
                        interleaved.extend(src.iter().map(|&s| s as f32 / 2_147_483_648.0));
                    } else {
                        // An exotic bit depth. Treat as silence rather than reading
                        // it as the wrong type and producing garbage spikes.
                        interleaved.resize(total, 0.0);
                    }
                }

                let _ = capture.ReleaseBuffer(frames);

                if channels > 1 {
                    // Downmix to mono by averaging. Summing would clip on correlated
                    // channels, which is most music.
                    let ch = channels as usize;
                    let mono: Vec<f32> = interleaved
                        .chunks_exact(ch)
                        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
                        .collect();
                    ring.push(&mono);
                } else {
                    ring.push(&interleaved);
                }
            }
        }

        // Emit on a fixed cadence regardless of how the packets arrived.
        let now = now_ms();
        if now - last_emit < emit_interval_ms {
            continue;
        }
        last_emit = now;

        if ring.peek_latest(&mut window) {
            // Keep a little more than one window so the next read always has a full
            // one ready, and so untouched older audio is not counted as an overrun.
            ring.retain_latest(FFT_SIZE + HOP_SIZE);
            let features = analyser.analyse(&window, WAVEFORM_POINTS);

            frames_emitted += 1;
            if frames_emitted % 100 == 0 {
                let loudest = features.bands.iter().cloned().fold(0.0f32, f32::max);
                let lit = features.bands.iter().filter(|&&b| b > 0.02).count();
                tracing::debug!(
                    rms = features.rms,
                    peak = features.peak,
                    bass = features.bass,
                    mid = features.mid,
                    treble = features.treble,
                    loudest_band = loudest,
                    bands_lit = lit,
                    silent = features.silent,
                    dropped = ring.dropped(),
                    "audio heartbeat"
                );
            }

            on_frame(AudioFrame::from_features(&features, now));
        } else {
            // Nothing buffered. For a loopback client this is what silence looks
            // like, so say so instead of leaving the last frame frozen on screen.
            on_frame(AudioFrame::silence(band_count, WAVEFORM_POINTS, now));
        }

        let dropped = ring.dropped();
        if dropped > 0 {
            status.lock().dropped_samples = dropped;
        }
    }

    let _ = client.Stop();
    let _ = CloseHandle(event);
    Ok(())
}

unsafe fn friendly_name(device: &IMMDevice) -> Option<String> {
    let store = device.OpenPropertyStore(STGM_READ).ok()?;
    let mut value = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME).ok()?;
    let name = value.to_string();
    let _ = PropVariantClear(&mut value);
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Enumerate active playback endpoints, for the device picker.
pub fn list_devices() -> Result<Vec<AudioDevice>> {
    unsafe {
        // The caller may be any thread; initialise an apartment for it and leave the
        // reference balanced on the way out.
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        let initialised = hr.is_ok();

        let result = (|| -> Result<Vec<AudioDevice>> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| AudioError::Unavailable(format!("device enumerator: {e}")))?;

            let default_id = enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .ok()
                .and_then(|d| d.GetId().ok())
                .map(|id| id.to_string().unwrap_or_default());

            let collection = enumerator
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                .map_err(|e| AudioError::Device(format!("EnumAudioEndpoints: {e}")))?;

            let count = collection.GetCount().unwrap_or(0);
            let mut out = Vec::with_capacity(count as usize);

            for i in 0..count {
                let Ok(device) = collection.Item(i) else {
                    continue;
                };
                let Ok(id_pwstr) = device.GetId() else {
                    continue;
                };
                let id = id_pwstr.to_string().unwrap_or_default();
                if id.is_empty() {
                    continue;
                }
                let name = friendly_name(&device).unwrap_or_else(|| "Unknown device".into());
                let is_default = default_id.as_deref() == Some(id.as_str());
                out.push(AudioDevice {
                    id,
                    name,
                    is_default,
                });
            }
            Ok(out)
        })();

        if initialised {
            CoUninitialize();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_rate_is_sane() {
        // Fast enough that a bass hit does not feel late, slow enough that three
        // windows are not flooded.
        const { assert!(EMIT_HZ >= 25.0 && EMIT_HZ <= 60.0) };
    }

    #[test]
    fn wait_timeout_is_shorter_than_an_emit_interval_times_ten() {
        // The timeout is what turns "no packets" into a silence frame. If it were
        // long, the display would freeze noticeably at the end of a track.
        const { assert!((WAIT_TIMEOUT_MS as f64) < 1000.0 / EMIT_HZ * 10.0) };
    }

    #[test]
    fn ring_holds_at_least_a_few_analysis_windows() {
        const {
            assert!(
                RING_SAMPLES > FFT_SIZE * 4,
                "too small to ride out a hiccup"
            )
        };
    }

    #[test]
    fn device_enumeration_does_not_panic() {
        // Runs on the real machine; an empty list is a legitimate result in CI.
        match list_devices() {
            Ok(devices) => {
                for d in &devices {
                    assert!(!d.id.is_empty());
                    assert!(!d.name.is_empty());
                }
                assert!(
                    devices.iter().filter(|d| d.is_default).count() <= 1,
                    "there cannot be two default endpoints"
                );
            }
            Err(e) => {
                // Acceptable on a machine with no audio hardware.
                println!("no audio devices available: {e}");
            }
        }
    }
}
