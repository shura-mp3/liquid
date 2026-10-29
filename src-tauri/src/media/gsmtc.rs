//! Windows Global System Media Transport Controls bridge.
//!
//! GSMTC (`Windows.Media.Control`) is the operating system's own media-session
//! facility — the same one that drives the volume-key popup. It reports metadata
//! uniformly for every media application that publishes a session, which is why
//! LIQUID needs no streaming-service API to know what is playing. Requires Windows
//! 10 build 17763+; the target is 19045.
//!
//! ## Timing: the `LastUpdatedTime` back-date
//!
//! The single most important detail in this file. `GetTimelineProperties()` returns
//! a `Position` **and** a `LastUpdatedTime` saying when the source sampled it. That
//! value can be seconds old. Timestamping it "now" would bake the staleness straight
//! into the lyric sync as a fixed error.
//!
//! So each observation is back-dated: `observed_at = monotonic_now - age`, where
//! `age` is how long ago the source sampled. The consumer then integrates forward
//! from the correct instant. This is what earns `TimingConfidence::Exact`.
//!
//! The age is computed from the wall clock, because `LastUpdatedTime` is a wall-clock
//! `DateTime` and there is no way to convert it to our monotonic base. That is safe
//! for a short interval but not for an arbitrary one, so the age is clamped: negative
//! ages (clock skew) become zero, and implausibly large ones fall back to "now" with
//! reduced confidence rather than producing a wildly wrong basis.
//!
//! ## Why there is still a refresh tick
//!
//! Change events (`MediaPropertiesChanged`, `PlaybackInfoChanged`,
//! `TimelinePropertiesChanged`, `CurrentSessionChanged`) fire on genuine changes, and
//! are subscribed to so a track switch is reflected immediately. But a source playing
//! steadily emits no timeline events for long stretches, so position would go stale.
//! A slow tick refreshes it: 1 s while playing, 5 s while paused, and the loop is
//! idle in between. That is bounded, cheap, and not the "poll the foreground window
//! at high frequency" pattern the project rules prohibit.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as WinPlaybackStatus,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

use super::{
    friendly_source_name, ControlCapabilities, MediaError, PlaybackSnapshot, PlaybackStatus,
    Provenance, Result, SessionSummary, TimingConfidence, TransportAction,
};
use crate::timing::MonotonicClock;

/// WinRT `DateTime` and `TimeSpan` are 100-nanosecond tick counts.
const TICKS_PER_MS: i64 = 10_000;

/// `DateTime` counts from 1601-01-01 UTC; Unix time counts from 1970-01-01.
const TICKS_1601_TO_1970: i64 = 116_444_736_000_000_000;

/// Beyond this, a `LastUpdatedTime` is not trustworthy enough to back-date from.
/// A source that last reported a position 30 s ago tells us nothing useful about
/// where it is now, so we stop pretending otherwise and drop the confidence.
const MAX_BACKDATE_MS: f64 = 30_000.0;

/// How often to refresh the timeline while playing.
const TICK_PLAYING: Duration = Duration::from_millis(1000);
/// A paused position does not move, so there is nothing to refresh quickly.
const TICK_IDLE: Duration = Duration::from_millis(5000);

/// Messages into the bridge thread.
enum Cmd {
    /// A WinRT event fired, or the tick elapsed: re-read and publish.
    Refresh,
    Transport(TransportAction),
    Seek(u64),
    Stop,
}

/// Shared, cheap-to-clone view of the latest state, for synchronous commands.
#[derive(Default)]
struct Shared {
    snapshot: Mutex<Option<PlaybackSnapshot>>,
    sessions: Mutex<Vec<SessionSummary>>,
    /// Source the user pinned, if any. Overrides arbitration.
    pinned: Mutex<Option<String>>,
    last_error: Mutex<Option<String>>,
    artwork: super::artwork::ArtworkCache,
}

pub struct GsmtcBridge {
    shared: Arc<Shared>,
    tx: Sender<Cmd>,
    running: Arc<AtomicBool>,
}

impl GsmtcBridge {
    /// Start the bridge thread. `on_change` is called whenever the published
    /// snapshot changes meaningfully, on the bridge thread.
    pub fn start<F>(clock: Arc<MonotonicClock>, on_change: F) -> Self
    where
        F: Fn(Option<PlaybackSnapshot>, Vec<SessionSummary>) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<Cmd>();
        let shared = Arc::new(Shared::default());
        let running = Arc::new(AtomicBool::new(true));

        let thread_shared = Arc::clone(&shared);
        let thread_running = Arc::clone(&running);
        let thread_tx = tx.clone();

        std::thread::Builder::new()
            .name("liquid-gsmtc".into())
            .spawn(move || {
                // WinRT needs an initialised apartment on this thread. MTA, because
                // we are a worker with no message pump.
                unsafe {
                    let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
                    if hr.is_err() {
                        tracing::error!(?hr, "CoInitializeEx failed; media sessions unavailable");
                        *thread_shared.last_error.lock() =
                            Some(format!("CoInitializeEx failed: {hr:?}"));
                        return;
                    }
                }

                run_bridge(
                    &thread_shared,
                    &thread_running,
                    &thread_tx,
                    rx,
                    &clock,
                    on_change,
                );

                unsafe { CoUninitialize() };
                tracing::info!("gsmtc bridge thread stopped");
            })
            .expect("could not spawn the gsmtc bridge thread");

        Self {
            shared,
            tx,
            running,
        }
    }

    pub fn snapshot(&self) -> Option<PlaybackSnapshot> {
        self.shared.snapshot.lock().clone()
    }

    pub fn sessions(&self) -> Vec<SessionSummary> {
        self.shared.sessions.lock().clone()
    }

    pub fn last_error(&self) -> Option<String> {
        self.shared.last_error.lock().clone()
    }

    /// Artwork for a key published in a snapshot. Fetched on the bridge thread when
    /// the track changed, so this is a cache lookup and never blocks on WinRT.
    pub fn artwork(&self, key: &str) -> Option<super::artwork::Artwork> {
        self.shared.artwork.get(key)
    }

    /// How many covers the bounded cache is holding. Surfaced in diagnostics so
    /// "no unbounded caches" is observable rather than merely claimed.
    pub fn artwork_entries(&self) -> usize {
        self.shared.artwork.len()
    }

    pub fn pinned(&self) -> Option<String> {
        self.shared.pinned.lock().clone()
    }

    /// Pin a source so arbitration always picks it, or clear the pin.
    pub fn set_pinned(&self, source_app: Option<String>) {
        *self.shared.pinned.lock() = source_app;
        let _ = self.tx.send(Cmd::Refresh);
    }

    pub fn transport(&self, action: TransportAction) -> Result<()> {
        // Refuse up front when the source said it cannot do this, so the caller gets
        // a reason rather than a call that quietly does nothing.
        let caps = self
            .shared
            .snapshot
            .lock()
            .as_ref()
            .map(|s| s.controls)
            .unwrap_or_default();

        let permitted = match action {
            TransportAction::Play => caps.play,
            TransportAction::Pause => caps.pause,
            TransportAction::PlayPause => caps.play || caps.pause,
            TransportAction::Next => caps.next,
            TransportAction::Previous => caps.previous,
        };
        if !permitted {
            return Err(MediaError::Unsupported(match action {
                TransportAction::Play => "play",
                TransportAction::Pause => "pause",
                TransportAction::PlayPause => "play/pause",
                TransportAction::Next => "skipping forward",
                TransportAction::Previous => "skipping back",
            }));
        }

        self.tx
            .send(Cmd::Transport(action))
            .map_err(|e| MediaError::Call(format!("bridge thread is gone: {e}")))
    }

    pub fn seek(&self, position_ms: u64) -> Result<()> {
        let can_seek = self
            .shared
            .snapshot
            .lock()
            .as_ref()
            .map(|s| s.controls.seek)
            .unwrap_or(false);
        if !can_seek {
            return Err(MediaError::Unsupported("seeking"));
        }
        self.tx
            .send(Cmd::Seek(position_ms))
            .map_err(|e| MediaError::Call(format!("bridge thread is gone: {e}")))
    }

    pub fn refresh(&self) {
        let _ = self.tx.send(Cmd::Refresh);
    }
}

impl Drop for GsmtcBridge {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = self.tx.send(Cmd::Stop);
    }
}

fn run_bridge<F>(
    shared: &Arc<Shared>,
    running: &Arc<AtomicBool>,
    tx: &Sender<Cmd>,
    rx: mpsc::Receiver<Cmd>,
    clock: &MonotonicClock,
    on_change: F,
) where
    F: Fn(Option<PlaybackSnapshot>, Vec<SessionSummary>) + Send + 'static,
{
    let manager = match SessionManager::RequestAsync().and_then(|op| op.join()) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "could not obtain the media session manager");
            *shared.last_error.lock() = Some(format!(
                "Windows media sessions unavailable: {e}. Requires Windows 10 build 17763 or newer."
            ));
            return;
        }
    };
    tracing::info!("media session manager acquired");

    // Manager-level events: a different app takes over, or the session list changes.
    let notify = tx.clone();
    let _current_token = manager.CurrentSessionChanged(&TypedEventHandler::new(move |_m, _a| {
        let _ = notify.send(Cmd::Refresh);
        Ok(())
    }));
    let notify = tx.clone();
    let _sessions_token = manager.SessionsChanged(&TypedEventHandler::new(move |_m, _a| {
        let _ = notify.send(Cmd::Refresh);
        Ok(())
    }));

    // Per-session event registrations are replaced whenever the current session
    // changes; holding them keeps the subscriptions alive.
    let mut subscriptions: Option<SessionSubscriptions> = None;
    let mut last_fingerprint: Option<_> = None;
    let mut current_source: Option<String> = None;
    let mut last_reason: Option<super::arbitration::Reason> = None;

    while running.load(Ordering::SeqCst) {
        let pinned = shared.pinned.lock().clone();
        // Summaries are built once per tick and reused for both the arbitration
        // decision and the UI's session list; each one costs a blocking WinRT call,
        // so building them twice would double that for no benefit.
        let sessions = summarise_sessions(&manager, pinned.as_deref());
        let picked = pick_session(&manager, &sessions, pinned.as_deref());
        let reason = picked.as_ref().map(|(_, r)| *r);
        let session = picked.map(|(s, _)| s);

        if reason != last_reason {
            tracing::debug!(?reason, "source arbitration changed");
            last_reason = reason;
        }

        // Re-subscribe when the chosen session changes.
        let source_now = session.as_ref().and_then(|s| source_app_of(s).ok());
        if source_now != current_source {
            subscriptions = session
                .as_ref()
                .map(|s| SessionSubscriptions::attach(s, tx.clone()));
            current_source = source_now;
            tracing::info!(source = ?current_source, "active media session changed");
        }

        let snapshot = session
            .as_ref()
            .and_then(|s| match read_snapshot(s, clock) {
                Ok(snap) => Some(snap),
                Err(e) => {
                    tracing::debug!(error = %e, "could not read media session");
                    None
                }
            });
        let fingerprint = snapshot.as_ref().map(|s| s.fingerprint());
        let changed = fingerprint != last_fingerprint;
        last_fingerprint = fingerprint;

        // Fetch artwork once per track, here on the bridge thread. Reading the
        // thumbnail is blocking WinRT I/O, so doing it lazily from a command would
        // stall the UI thread; doing it every tick would re-read the same image
        // repeatedly. The cache key only changes when the track does.
        if let (Some(snap), Some(sess)) = (&snapshot, &session) {
            if let Some(key) = snap.artwork_key.as_deref() {
                if shared.artwork.get(key).is_none() {
                    match read_thumbnail(sess) {
                        Ok(art) => {
                            tracing::debug!(key, bytes = art.bytes.len(), mime = %art.mime, "artwork fetched");
                            shared.artwork.insert(key.to_string(), art);
                        }
                        Err(e) => tracing::debug!(key, error = %e, "no artwork for this track"),
                    }
                }
            }
        }

        let playing = snapshot
            .as_ref()
            .map(|s| s.status == PlaybackStatus::Playing)
            .unwrap_or(false);

        *shared.snapshot.lock() = snapshot.clone();
        *shared.sessions.lock() = sessions.clone();
        if changed {
            if let Some(s) = &snapshot {
                tracing::info!(
                    title = %s.title,
                    artist = ?s.artist,
                    album = ?s.album,
                    source = ?s.source_app,
                    status = ?s.status,
                    duration_ms = ?s.duration_ms,
                    position_ms = s.position_ms,
                    // Capabilities and timing are logged because they are exactly
                    // what has to be verified rather than assumed.
                    play = s.controls.play,
                    pause = s.controls.pause,
                    next = s.controls.next,
                    previous = s.controls.previous,
                    seek = s.controls.seek,
                    timing = ?s.timing_confidence,
                    age_ms = s.age_ms,
                    artwork = s.artwork_key.is_some(),
                    "playback changed"
                );
            } else {
                tracing::info!("no active media session");
            }
        }
        // Always publish: position advances even when the fingerprint is stable, and
        // the consumer's clock wants fresh observations to correct drift against.
        on_change(snapshot, sessions);

        let wait = if playing { TICK_PLAYING } else { TICK_IDLE };
        match rx.recv_timeout(wait) {
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Cmd::Refresh) | Err(RecvTimeoutError::Timeout) => {}
            Ok(Cmd::Transport(action)) => {
                if let Some(s) = &session {
                    if let Err(e) = apply_transport(s, action) {
                        tracing::warn!(?action, error = %e, "transport command failed");
                    }
                }
            }
            Ok(Cmd::Seek(position_ms)) => {
                if let Some(s) = &session {
                    let ticks = (position_ms as i64).saturating_mul(TICKS_PER_MS);
                    match s
                        .TryChangePlaybackPositionAsync(ticks)
                        .and_then(|op| op.join())
                    {
                        Ok(true) => tracing::info!(position_ms, "seek accepted"),
                        Ok(false) => tracing::warn!(position_ms, "source refused the seek"),
                        Err(e) => tracing::warn!(error = %e, "seek failed"),
                    }
                }
            }
        }
    }

    drop(subscriptions);
}

/// Event registrations for one session, unregistered on drop.
struct SessionSubscriptions {
    session: Session,
    tokens: Vec<(SubKind, i64)>,
}

#[derive(Clone, Copy)]
enum SubKind {
    Media,
    Playback,
    Timeline,
}

impl SessionSubscriptions {
    fn attach(session: &Session, tx: Sender<Cmd>) -> Self {
        let mut tokens = Vec::new();

        let notify = tx.clone();
        if let Ok(t) = session.MediaPropertiesChanged(&TypedEventHandler::new(move |_s, _a| {
            let _ = notify.send(Cmd::Refresh);
            Ok(())
        })) {
            tokens.push((SubKind::Media, t));
        }

        let notify = tx.clone();
        if let Ok(t) = session.PlaybackInfoChanged(&TypedEventHandler::new(move |_s, _a| {
            let _ = notify.send(Cmd::Refresh);
            Ok(())
        })) {
            tokens.push((SubKind::Playback, t));
        }

        let notify = tx;
        if let Ok(t) = session.TimelinePropertiesChanged(&TypedEventHandler::new(move |_s, _a| {
            let _ = notify.send(Cmd::Refresh);
            Ok(())
        })) {
            tokens.push((SubKind::Timeline, t));
        }

        Self {
            session: session.clone(),
            tokens,
        }
    }
}

impl Drop for SessionSubscriptions {
    fn drop(&mut self) {
        for (kind, token) in &self.tokens {
            // Registration tokens are plain i64 in windows-rs 0.62.
            let _ = match kind {
                SubKind::Media => self.session.RemoveMediaPropertiesChanged(*token),
                SubKind::Playback => self.session.RemovePlaybackInfoChanged(*token),
                SubKind::Timeline => self.session.RemoveTimelinePropertiesChanged(*token),
            };
        }
    }
}

fn source_app_of(session: &Session) -> windows::core::Result<String> {
    Ok(session.SourceAppUserModelId()?.to_string())
}

/// Resolve the arbitration decision to a live session object.
///
/// The decision itself lives in `arbitration::choose`, which is pure and
/// exhaustively tested. Reimplementing the priority order here against live WinRT
/// objects would mean two rule sets that must agree forever — the same trap that
/// keeps track-identity normalisation in one place.
fn pick_session(
    manager: &SessionManager,
    summaries: &[SessionSummary],
    pinned: Option<&str>,
) -> Option<(Session, super::arbitration::Reason)> {
    let choice = super::arbitration::choose(summaries, pinned)?;

    let session = manager.GetSessions().ok()?.into_iter().find(|s| {
        source_app_of(s)
            .map(|id| id == choice.source_app)
            .unwrap_or(false)
    })?;

    Some((session, choice.reason))
}

fn status_of(session: &Session) -> PlaybackStatus {
    match session.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()) {
        Ok(WinPlaybackStatus::Playing) => PlaybackStatus::Playing,
        Ok(WinPlaybackStatus::Paused) => PlaybackStatus::Paused,
        Ok(WinPlaybackStatus::Stopped) | Ok(WinPlaybackStatus::Closed) => PlaybackStatus::Stopped,
        _ => PlaybackStatus::Unknown,
    }
}

fn summarise_sessions(manager: &SessionManager, pinned: Option<&str>) -> Vec<SessionSummary> {
    let current_id = manager
        .GetCurrentSession()
        .ok()
        .and_then(|s| source_app_of(&s).ok());

    manager
        .GetSessions()
        .ok()
        .map(|list| {
            list.into_iter()
                .filter_map(|s| {
                    let source_app = source_app_of(&s).ok()?;
                    // Metadata may legitimately be unavailable; the session still
                    // belongs in the picker so the user can choose it.
                    let props = s.TryGetMediaPropertiesAsync().and_then(|op| op.join()).ok();
                    Some(SessionSummary {
                        display_name: friendly_source_name(&source_app),
                        title: props
                            .as_ref()
                            .and_then(|p| p.Title().ok())
                            .map(|t| t.to_string())
                            .filter(|t| !t.is_empty()),
                        artist: props
                            .as_ref()
                            .and_then(|p| p.Artist().ok())
                            .map(|t| t.to_string())
                            .filter(|t| !t.is_empty()),
                        status: status_of(&s),
                        is_current: current_id.as_deref() == Some(source_app.as_str()),
                        is_pinned: pinned == Some(source_app.as_str()),
                        source_app,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Milliseconds since the Unix epoch, as an f64.
fn wall_clock_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn read_snapshot(session: &Session, clock: &MonotonicClock) -> Result<PlaybackSnapshot> {
    let props = session
        .TryGetMediaPropertiesAsync()
        .and_then(|op| op.join())
        .map_err(|e| MediaError::Call(format!("TryGetMediaPropertiesAsync: {e}")))?;

    let title = props
        .Title()
        .map(|t| t.to_string())
        .unwrap_or_default()
        .trim()
        .to_string();

    // A session with no title is not showing anything useful; treat it as nothing
    // rather than publishing an empty widget.
    if title.is_empty() {
        return Err(MediaError::NoSession);
    }

    let non_empty = |s: String| if s.trim().is_empty() { None } else { Some(s) };
    let artist = props
        .Artist()
        .ok()
        .map(|t| t.to_string())
        .and_then(non_empty);
    let album = props
        .AlbumTitle()
        .ok()
        .map(|t| t.to_string())
        .and_then(non_empty);

    let info = session
        .GetPlaybackInfo()
        .map_err(|e| MediaError::Call(format!("GetPlaybackInfo: {e}")))?;

    let status = match info.PlaybackStatus() {
        Ok(WinPlaybackStatus::Playing) => PlaybackStatus::Playing,
        Ok(WinPlaybackStatus::Paused) => PlaybackStatus::Paused,
        Ok(WinPlaybackStatus::Stopped) | Ok(WinPlaybackStatus::Closed) => PlaybackStatus::Stopped,
        _ => PlaybackStatus::Unknown,
    };

    // Rate is an IReference and is frequently null. Null means "normal", not "zero".
    let rate = info
        .PlaybackRate()
        .ok()
        .and_then(|r| r.Value().ok())
        .filter(|r| *r > 0.0)
        .unwrap_or(1.0);

    let controls = info
        .Controls()
        .map(|c| ControlCapabilities {
            play: c.IsPlayEnabled().unwrap_or(false),
            pause: c.IsPauseEnabled().unwrap_or(false),
            next: c.IsNextEnabled().unwrap_or(false),
            previous: c.IsPreviousEnabled().unwrap_or(false),
            seek: c.IsPlaybackPositionEnabled().unwrap_or(false),
            // GSMTC exposes no volume control at all, so this is always false
            // rather than optimistically true.
            volume: false,
        })
        .unwrap_or_default();

    let timeline = session
        .GetTimelineProperties()
        .map_err(|e| MediaError::Call(format!("GetTimelineProperties: {e}")))?;

    let position_ticks = timeline.Position().map(|t| t.Duration).unwrap_or(0);
    let end_ticks = timeline.EndTime().map(|t| t.Duration).unwrap_or(0);
    let start_ticks = timeline.StartTime().map(|t| t.Duration).unwrap_or(0);

    let position_ms = (position_ticks.max(0) / TICKS_PER_MS) as u64;
    // Duration is EndTime - StartTime; some sources report a non-zero start.
    let duration_ms = match end_ticks.saturating_sub(start_ticks) {
        d if d > 0 => Some((d / TICKS_PER_MS) as u64),
        _ => None,
    };

    // --- the back-date, see the module docs -------------------------------------
    let now_mono = clock.now_ms();
    let (observed_at_ms, timing_confidence) = match timeline.LastUpdatedTime() {
        Ok(dt) if dt.UniversalTime > 0 => {
            let updated_unix_ms =
                (dt.UniversalTime - TICKS_1601_TO_1970) as f64 / TICKS_PER_MS as f64;
            let age_ms = wall_clock_ms() - updated_unix_ms;
            tracing::debug!(
                ticks = dt.UniversalTime,
                updated_unix_ms,
                age_ms,
                "timeline LastUpdatedTime"
            );
            if age_ms.is_finite() && (0.0..=MAX_BACKDATE_MS).contains(&age_ms) {
                (now_mono - age_ms, TimingConfidence::Exact)
            } else if age_ms.is_finite() && age_ms > MAX_BACKDATE_MS {
                // The source published a timeline once and has not touched it since.
                // Observed in the wild: Brave kept `LastUpdatedTime` fixed for 41
                // minutes while a video played, with `Position` frozen at 13 ms.
                //
                // This is NOT merely "less precise". The position is not advancing at
                // all, so interpolating from it would confidently show the opening
                // lyrics of a track that is three quarters finished. Reported as
                // `Unknown` so the UI can say the source does not report position,
                // rather than quietly displaying something wrong.
                (now_mono, TimingConfidence::Unknown)
            } else {
                // Negative or non-finite: clock skew between the source's wall clock
                // and ours. The position itself is probably fine, just unanchorable.
                (now_mono, TimingConfidence::Coarse)
            }
        }
        Ok(dt) => {
            tracing::debug!(ticks = dt.UniversalTime, "timeline has no LastUpdatedTime");
            (now_mono, TimingConfidence::Coarse)
        }
        Err(e) => {
            tracing::debug!(error = %e, "LastUpdatedTime unavailable");
            (now_mono, TimingConfidence::Coarse)
        }
    };

    let source_app = source_app_of(session).ok().filter(|s| !s.is_empty());

    // Artwork is fetched lazily by key; the key changes when the track does.
    let artwork_key = props
        .Thumbnail()
        .ok()
        .map(|_| super::artwork::key_for(&title, artist.as_deref(), album.as_deref()));

    Ok(PlaybackSnapshot {
        title,
        artist,
        album,
        duration_ms,
        position_ms,
        status,
        rate,
        observed_at_ms,
        age_ms: (now_mono - observed_at_ms).max(0.0),
        timing_confidence,
        controls,
        source_app,
        provenance: Provenance::Gsmtc,
        artwork_key,
    })
}

/// Read the current session's thumbnail into bytes.
///
/// Blocking WinRT I/O, so this must run on the bridge thread, never on the UI
/// thread. The declared content type is ignored in favour of sniffing the magic
/// bytes: sources report it inconsistently or leave it empty, and a data URL with
/// the wrong MIME simply fails to render.
pub fn read_thumbnail(session: &Session) -> Result<super::artwork::Artwork> {
    use windows::Storage::Streams::{DataReader, InputStreamOptions};

    let reference = session
        .TryGetMediaPropertiesAsync()
        .and_then(|op| op.join())
        .and_then(|props| props.Thumbnail())
        .map_err(|e| MediaError::Call(format!("no thumbnail: {e}")))?;

    let stream = reference
        .OpenReadAsync()
        .and_then(|op| op.join())
        .map_err(|e| MediaError::Call(format!("OpenReadAsync: {e}")))?;

    let size = stream
        .Size()
        .map_err(|e| MediaError::Call(format!("stream size: {e}")))?;
    if size == 0 {
        return Err(MediaError::Call("thumbnail stream is empty".into()));
    }
    // Guard before allocating: the size comes from outside this process.
    if size > super::artwork::MAX_BYTES as u64 {
        return Err(MediaError::Call(format!(
            "thumbnail is {size} bytes, over the {} byte limit",
            super::artwork::MAX_BYTES
        )));
    }

    let reader = DataReader::CreateDataReader(&stream)
        .map_err(|e| MediaError::Call(format!("CreateDataReader: {e}")))?;
    let _ = reader.SetInputStreamOptions(InputStreamOptions::ReadAhead);

    let loaded = reader
        .LoadAsync(size as u32)
        .and_then(|op| op.join())
        .map_err(|e| MediaError::Call(format!("LoadAsync: {e}")))?;

    let mut bytes = vec![0u8; loaded as usize];
    reader
        .ReadBytes(&mut bytes)
        .map_err(|e| MediaError::Call(format!("ReadBytes: {e}")))?;

    let mime = super::artwork::sniff_mime(&bytes);
    if mime == "application/octet-stream" {
        return Err(MediaError::Call(
            "thumbnail is not a recognised image format".into(),
        ));
    }

    Ok(super::artwork::Artwork {
        mime: mime.to_string(),
        bytes,
    })
}

fn apply_transport(session: &Session, action: TransportAction) -> windows::core::Result<bool> {
    match action {
        TransportAction::Play => session.TryPlayAsync()?.join(),
        TransportAction::Pause => session.TryPauseAsync()?.join(),
        TransportAction::PlayPause => session.TryTogglePlayPauseAsync()?.join(),
        TransportAction::Next => session.TrySkipNextAsync()?.join(),
        TransportAction::Previous => session.TrySkipPreviousAsync()?.join(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_intervals_are_sane() {
        // Playing must refresh faster than idle, and neither may be a busy loop.
        assert!(TICK_PLAYING < TICK_IDLE);
        assert!(TICK_PLAYING >= Duration::from_millis(250));
    }

    #[test]
    fn epoch_offset_converts_a_known_datetime() {
        // 1970-01-01T00:00:00Z expressed in DateTime ticks must map to Unix 0.
        let unix_ms = (TICKS_1601_TO_1970 - TICKS_1601_TO_1970) as f64 / TICKS_PER_MS as f64;
        assert_eq!(unix_ms, 0.0);

        // One day after the Unix epoch.
        let one_day_ticks = TICKS_1601_TO_1970 + 86_400_000 * TICKS_PER_MS;
        let ms = (one_day_ticks - TICKS_1601_TO_1970) as f64 / TICKS_PER_MS as f64;
        assert_eq!(ms, 86_400_000.0);
    }

    #[test]
    fn backdate_window_rejects_an_implausible_age() {
        assert!(
            !(0.0..=MAX_BACKDATE_MS).contains(&-50.0),
            "negative age must be rejected"
        );
        assert!(
            !(0.0..=MAX_BACKDATE_MS).contains(&60_000.0),
            "a minute-old sample must be rejected"
        );
        assert!(
            (0.0..=MAX_BACKDATE_MS).contains(&250.0),
            "a fresh sample must be accepted"
        );
    }
}
