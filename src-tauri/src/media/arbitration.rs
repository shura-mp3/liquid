//! Which source wins when several are playing.
//!
//! Pure decision logic, separated from the WinRT plumbing so it can be tested
//! exhaustively without a media session in the room. `gsmtc.rs` applies the same
//! order against live sessions.
//!
//! The rule that matters most is the one about audio: LIQUID analyses the Windows
//! system mix and cannot attribute it to a process, so it must **never** infer that
//! the loudest thing it hears is the session it is displaying. Arbitration is based
//! only on what the OS reports about sessions.

use super::{PlaybackStatus, SessionSummary};

/// In priority order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The user pinned this source explicitly.
    Pinned,
    /// The OS's current session, and it is playing.
    CurrentAndPlaying,
    /// Something else is playing while the current session is not.
    OnlyOnePlaying,
    /// Nothing is playing; fall back to what the OS considers current.
    CurrentFallback,
    /// Nothing is playing and there is no current session.
    FirstAvailable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub source_app: String,
    pub reason: Reason,
}

/// Pick the session to follow.
///
/// A pinned source wins even while paused — the user said they want that one, and
/// silently switching away from an explicit choice is worse than showing it paused.
/// A pin naming a source that is no longer present is ignored rather than leaving
/// the HUD blank.
pub fn choose(sessions: &[SessionSummary], pinned: Option<&str>) -> Option<Choice> {
    if sessions.is_empty() {
        return None;
    }

    if let Some(pin) = pinned {
        if let Some(found) = sessions.iter().find(|s| s.source_app == pin) {
            return Some(Choice {
                source_app: found.source_app.clone(),
                reason: Reason::Pinned,
            });
        }
    }

    let playing: Vec<&SessionSummary> = sessions
        .iter()
        .filter(|s| s.status == PlaybackStatus::Playing)
        .collect();

    if let Some(current) = sessions.iter().find(|s| s.is_current) {
        if current.status == PlaybackStatus::Playing {
            return Some(Choice {
                source_app: current.source_app.clone(),
                reason: Reason::CurrentAndPlaying,
            });
        }
        // The current session is idle. If exactly one other source is actually
        // playing, follow it — a paused browser tab should not mask live music.
        // With several playing we cannot tell which the user means, so we stay on
        // the OS's choice rather than picking arbitrarily.
        if playing.len() == 1 {
            return Some(Choice {
                source_app: playing[0].source_app.clone(),
                reason: Reason::OnlyOnePlaying,
            });
        }
        return Some(Choice {
            source_app: current.source_app.clone(),
            reason: Reason::CurrentFallback,
        });
    }

    if playing.len() == 1 {
        return Some(Choice {
            source_app: playing[0].source_app.clone(),
            reason: Reason::OnlyOnePlaying,
        });
    }

    sessions.first().map(|s| Choice {
        source_app: s.source_app.clone(),
        reason: Reason::FirstAvailable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(app: &str, status: PlaybackStatus, is_current: bool) -> SessionSummary {
        SessionSummary {
            source_app: app.into(),
            display_name: app.into(),
            title: Some("Track".into()),
            artist: Some("Artist".into()),
            status,
            is_current,
            is_pinned: false,
        }
    }

    #[test]
    fn returns_nothing_when_there_are_no_sessions() {
        assert!(choose(&[], None).is_none());
        assert!(choose(&[], Some("Spotify.exe")).is_none());
    }

    #[test]
    fn a_pin_wins_over_everything() {
        let sessions = vec![
            session("Spotify.exe", PlaybackStatus::Paused, false),
            session("msedge.exe", PlaybackStatus::Playing, true),
        ];
        let choice = choose(&sessions, Some("Spotify.exe")).unwrap();
        assert_eq!(choice.source_app, "Spotify.exe");
        assert_eq!(choice.reason, Reason::Pinned);
    }

    #[test]
    fn a_stale_pin_is_ignored_rather_than_blanking_the_hud() {
        let sessions = vec![session("msedge.exe", PlaybackStatus::Playing, true)];
        let choice = choose(&sessions, Some("Spotify.exe")).unwrap();
        assert_eq!(choice.source_app, "msedge.exe");
        assert_eq!(choice.reason, Reason::CurrentAndPlaying);
    }

    #[test]
    fn prefers_the_current_session_when_it_is_playing() {
        let sessions = vec![
            session("Spotify.exe", PlaybackStatus::Playing, true),
            session("msedge.exe", PlaybackStatus::Playing, false),
        ];
        let choice = choose(&sessions, None).unwrap();
        assert_eq!(choice.source_app, "Spotify.exe");
        assert_eq!(choice.reason, Reason::CurrentAndPlaying);
    }

    #[test]
    fn follows_the_only_playing_source_when_the_current_one_is_paused() {
        let sessions = vec![
            session("msedge.exe", PlaybackStatus::Paused, true),
            session("Spotify.exe", PlaybackStatus::Playing, false),
        ];
        let choice = choose(&sessions, None).unwrap();
        assert_eq!(choice.source_app, "Spotify.exe");
        assert_eq!(choice.reason, Reason::OnlyOnePlaying);
    }

    #[test]
    fn stays_on_the_current_session_when_several_others_are_playing() {
        // Ambiguous: picking one arbitrarily would flip around unpredictably.
        let sessions = vec![
            session("msedge.exe", PlaybackStatus::Paused, true),
            session("Spotify.exe", PlaybackStatus::Playing, false),
            session("vlc.exe", PlaybackStatus::Playing, false),
        ];
        let choice = choose(&sessions, None).unwrap();
        assert_eq!(choice.source_app, "msedge.exe");
        assert_eq!(choice.reason, Reason::CurrentFallback);
    }

    #[test]
    fn falls_back_to_the_current_session_when_nothing_is_playing() {
        let sessions = vec![
            session("Spotify.exe", PlaybackStatus::Paused, true),
            session("msedge.exe", PlaybackStatus::Stopped, false),
        ];
        let choice = choose(&sessions, None).unwrap();
        assert_eq!(choice.source_app, "Spotify.exe");
        assert_eq!(choice.reason, Reason::CurrentFallback);
    }

    #[test]
    fn picks_the_playing_source_when_the_os_reports_no_current_session() {
        let sessions = vec![
            session("Spotify.exe", PlaybackStatus::Paused, false),
            session("vlc.exe", PlaybackStatus::Playing, false),
        ];
        let choice = choose(&sessions, None).unwrap();
        assert_eq!(choice.source_app, "vlc.exe");
        assert_eq!(choice.reason, Reason::OnlyOnePlaying);
    }

    #[test]
    fn falls_back_to_the_first_session_when_nothing_distinguishes_them() {
        let sessions = vec![
            session("a.exe", PlaybackStatus::Paused, false),
            session("b.exe", PlaybackStatus::Paused, false),
        ];
        let choice = choose(&sessions, None).unwrap();
        assert_eq!(choice.source_app, "a.exe");
        assert_eq!(choice.reason, Reason::FirstAvailable);
    }

    #[test]
    fn a_pinned_paused_source_is_still_honoured() {
        let sessions = vec![
            session("Spotify.exe", PlaybackStatus::Paused, false),
            session("vlc.exe", PlaybackStatus::Playing, true),
        ];
        let choice = choose(&sessions, Some("Spotify.exe")).unwrap();
        assert_eq!(choice.reason, Reason::Pinned);
    }
}
