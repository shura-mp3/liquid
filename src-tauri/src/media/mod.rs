//! Music provider abstraction.
//!
//! One normalised shape for playback state, whatever produced it. The types mirror
//! `src/core/types.ts` field for field, because the same observation crosses the IPC
//! boundary and is fed to the TypeScript playback clock, which is where
//! interpolation happens (it is already built and tested, so the native side emits
//! observations rather than duplicating the prediction logic).
//!
//! Everything optional here is genuinely optional. Real sources omit album, artwork
//! and duration constantly, and the rule is that absence is rendered as absence —
//! nothing is invented to fill a gap.

use serde::{Deserialize, Serialize};

pub mod arbitration;
pub mod artwork;

#[cfg(windows)]
pub mod gsmtc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
    Unknown,
}

/// How much the reported position can be trusted *at the moment it was observed*.
/// This is about the source, not about our own prediction drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimingConfidence {
    /// The source told us exactly when it sampled the position, and we back-dated
    /// our observation to that instant. GSMTC does this via `LastUpdatedTime`.
    Exact,
    Interpolated,
    Coarse,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provenance {
    Gsmtc,
    Extension,
    Synthetic,
    User,
}

/// What the source has actually advertised it can do. Never assumed, never defaulted
/// to "everything": a disabled Seek button must mean *this source said it cannot
/// seek*, which is what lets the UI explain itself instead of failing silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlCapabilities {
    pub play: bool,
    pub pause: bool,
    pub next: bool,
    pub previous: bool,
    pub seek: bool,
    pub volume: bool,
}

/// Raw playback state as the source reports it.
///
/// Deliberately carries **no computed track identity**. Normalising a title into a
/// stable identity (stripping upload noise, keeping version markers, bucketing
/// duration) already exists in `src/core/identity.ts` with an exhaustive test suite,
/// and that is also where the playback clock and the lyrics matcher consume it.
/// Reimplementing it here would mean two normalisers that must agree forever and
/// would silently mismatch offsets the day they drift apart. So the native side
/// reports facts, and the single tested implementation derives identity from them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackSnapshot {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub position_ms: u64,
    pub status: PlaybackStatus,
    pub rate: f64,
    /// Our monotonic clock, in ms. Back-dated to when the *source* sampled the
    /// position where that is knowable, so the consumer's interpolation starts from
    /// the right instant rather than from when we happened to ask.
    ///
    /// This is on the **native** clock base, which the webview cannot read — use
    /// `age_ms` to convert.
    pub observed_at_ms: f64,
    /// How long before this snapshot was produced the position was sampled.
    ///
    /// The webview converts with `observedAt = performance.now() - ageMs`, because
    /// an age is comparable across two unrelated monotonic clocks while an absolute
    /// timestamp is not. A clock-synchronisation handshake would be the alternative;
    /// this needs no protocol and no drift correction. The only error is IPC transit
    /// (a millisecond or two), which the consumer's uncertainty model absorbs.
    pub age_ms: f64,
    pub timing_confidence: TimingConfidence,
    pub controls: ControlCapabilities,
    /// AUMID or executable name, e.g. `Spotify.exe`.
    pub source_app: Option<String>,
    pub provenance: Provenance,
    /// Cache key for artwork fetched separately, absent when the source has none.
    pub artwork_key: Option<String>,
}

impl PlaybackSnapshot {
    /// Cheap equality key for "has anything meaningful changed".
    ///
    /// Only used to decide whether to emit an update; it is not a track identity and
    /// must never be persisted or used to look up a lyric offset. Position is
    /// excluded on purpose, since it changes constantly by design.
    pub fn fingerprint(
        &self,
    ) -> (
        String,
        Option<String>,
        Option<u64>,
        PlaybackStatus,
        Option<String>,
    ) {
        (
            self.title.clone(),
            self.artist.clone(),
            self.duration_ms,
            self.status,
            self.source_app.clone(),
        )
    }
}

/// A media session as the OS reports it, for the source picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub source_app: String,
    /// Friendly name derived from the AUMID where we recognise it.
    pub display_name: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub status: PlaybackStatus,
    pub is_current: bool,
    pub is_pinned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportAction {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("no media session is active")]
    NoSession,
    #[error("this source does not support {0}")]
    Unsupported(&'static str),
    #[error("media session call failed: {0}")]
    Call(String),
}

pub type Result<T> = std::result::Result<T, MediaError>;

/// Map an AUMID or executable name to something a human recognises.
///
/// Deliberately a small, explicit table rather than a guess: an unknown source keeps
/// its raw identifier, which is more useful than a mangled approximation.
pub fn friendly_source_name(source_app: &str) -> String {
    let lower = source_app.to_ascii_lowercase();
    let known = [
        ("spotify", "Spotify"),
        ("msedge", "Microsoft Edge"),
        ("chrome", "Google Chrome"),
        ("firefox", "Firefox"),
        ("zen.", "Zen Browser"),
        ("brave", "Brave"),
        ("opera", "Opera"),
        ("vivaldi", "Vivaldi"),
        ("vlc", "VLC"),
        ("foobar2000", "foobar2000"),
        ("musicbee", "MusicBee"),
        ("aimp", "AIMP"),
        ("winamp", "Winamp"),
        ("itunes", "iTunes"),
        ("applemusic", "Apple Music"),
        ("zunemusic", "Groove Music"),
        ("media.player", "Media Player"),
        ("tidal", "TIDAL"),
        ("deezer", "Deezer"),
        ("youtube", "YouTube Music"),
    ];
    for (needle, name) in known {
        if lower.contains(needle) {
            return (*name).to_string();
        }
    }
    // Strip the UWP package-family suffix so at least the readable part survives.
    source_app
        .split('!')
        .next()
        .unwrap_or(source_app)
        .trim_end_matches(".exe")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_common_sources() {
        assert_eq!(friendly_source_name("Spotify.exe"), "Spotify");
        assert_eq!(friendly_source_name("msedge.exe"), "Microsoft Edge");
        assert_eq!(friendly_source_name("chrome.exe"), "Google Chrome");
        assert_eq!(friendly_source_name("foobar2000.exe"), "foobar2000");
    }

    #[test]
    fn keeps_an_unknown_identifier_readable_instead_of_guessing() {
        assert_eq!(
            friendly_source_name("SomeObscurePlayer.exe"),
            "SomeObscurePlayer"
        );
    }

    #[test]
    fn strips_a_uwp_package_suffix() {
        assert_eq!(
            friendly_source_name("28119AlexanderSklyarov.Unknown_abc!App"),
            "28119AlexanderSklyarov.Unknown_abc"
        );
    }

    #[test]
    fn capabilities_default_to_nothing_allowed() {
        let c = ControlCapabilities::default();
        assert!(!c.play && !c.pause && !c.next && !c.previous && !c.seek && !c.volume);
    }
}
