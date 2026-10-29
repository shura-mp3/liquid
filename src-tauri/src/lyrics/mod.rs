//! Lyrics acquisition.
//!
//! ## What lives here, and what deliberately does not
//!
//! This module does **I/O and politeness**: HTTP to LRCLIB, rate limiting, backoff,
//! the on-disk cache, and reading local `.lrc` files. It does **not** parse LRC and
//! it does **not** decide which candidate wins.
//!
//! Both of those already exist in TypeScript — `src/core/lrc.ts` (36 tests covering
//! BOMs, CRLF, multi-timestamp lines, Enhanced-LRC word timings, positional
//! fractions) and `src/core/matcher.ts` (23 tests covering the duration gate and the
//! live/remix guard). Reimplementing either here would create two implementations
//! that must agree forever, which is the same trap avoided for track identity. So
//! raw lyric text crosses the IPC boundary unparsed and the tested code handles it.
//!
//! ## Why `syncedLyrics` rather than `lyricsfile`
//!
//! LRCLIB records carry both. `lyricsfile` is a YAML document whose schema is only
//! partially documented; an observed v1.0 record looks like:
//!
//! ```yaml
//! version: '1.0'
//! metadata: { title: ..., artist: ..., duration_ms: 355000, instrumental: false }
//! lines:
//! - text: Is this the real life? Is this just fantasy?
//!   start_ms: 150
//!   end_ms: 7130
//! ```
//!
//! That `end_ms` is genuinely richer than LRC, which only has line starts. But word
//! timings are documented as living under `lines[].words` with no stated shape, and
//! across several searches of the live API **no record with `hasWordSync: true` was
//! found at all** — so the word schema could not be verified against real data.
//! Parsing it would also mean adding a YAML dependency, and the obvious candidate
//! (`serde_yaml`) is unmaintained.
//!
//! `syncedLyrics` is present on every synced record, is ordinary LRC, and is already
//! parsed exhaustively. Line end times are derived from the next line's start, which
//! is what LRC consumers do anyway. Word-level karaoke therefore comes from Enhanced
//! LRC (`<mm:ss.xx>` markers) in local files and user-authored lyrics — a real,
//! verified format — rather than from an inferred one.

use serde::{Deserialize, Serialize};

pub mod cache;
pub mod local;
pub mod lrclib;
pub mod service;

#[derive(Debug, thiserror::Error)]
pub enum LyricsError {
    #[error("network request failed: {0}")]
    Network(String),
    #[error("lyrics provider returned {status}{}", .retry_after_s.map(|s| format!(", retry after {s}s")).unwrap_or_default())]
    Http {
        status: u16,
        retry_after_s: Option<u64>,
    },
    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("database error: {0}")]
    Db(String),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, LyricsError>;

/// What LIQUID asks a provider for.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsQuery {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
}

/// A candidate as returned by a provider, unparsed and unjudged.
///
/// Field names mirror LRCLIB's own so the shape is traceable to the API, and mirror
/// `LyricsCandidate` in `src/core/matcher.ts` so scoring needs no translation layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsCandidate {
    pub id: i64,
    pub track_name: String,
    pub artist_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album_name: Option<String>,
    /// Seconds, as LRCLIB reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default)]
    pub has_word_sync: bool,
    /// Raw LRC. Parsed on the TypeScript side.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synced_lyrics: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plain_lyrics: Option<String>,
}

/// Where a set of lyrics came from. Shown in the UI, because the user should be able
/// to tell a local file from a network result from their own edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LyricsOrigin {
    Cache,
    Lrclib,
    LocalFile,
    UserEdited,
}

/// A resolved set of lyrics, ready for the frontend to parse and display.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedLyrics {
    /// Raw LRC (or plain text) exactly as the source provided it.
    pub text: String,
    pub origin: LyricsOrigin,
    pub instrumental: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<i64>,
    /// Shown as attribution wherever the lyrics appear.
    pub attribution: Option<String>,
}

/// Attribution text for a provider. LRCLIB's docs state no licence terms, so LIQUID
/// credits it visibly on everything fetched and caches rather than re-requesting.
pub fn attribution_for(origin: LyricsOrigin) -> Option<String> {
    match origin {
        LyricsOrigin::Lrclib | LyricsOrigin::Cache => Some("Lyrics from LRCLIB".to_string()),
        LyricsOrigin::LocalFile => None,
        LyricsOrigin::UserEdited => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_results_are_attributed() {
        assert_eq!(
            attribution_for(LyricsOrigin::Lrclib).as_deref(),
            Some("Lyrics from LRCLIB")
        );
        // A cached result is still a network result and keeps its credit.
        assert_eq!(
            attribution_for(LyricsOrigin::Cache).as_deref(),
            Some("Lyrics from LRCLIB")
        );
    }

    #[test]
    fn the_users_own_files_are_not_attributed_to_a_provider() {
        assert!(attribution_for(LyricsOrigin::LocalFile).is_none());
        assert!(attribution_for(LyricsOrigin::UserEdited).is_none());
    }

    #[test]
    fn http_error_mentions_retry_after_when_known() {
        let e = LyricsError::Http {
            status: 429,
            retry_after_s: Some(12),
        };
        assert!(e.to_string().contains("429"));
        assert!(e.to_string().contains("12"));
    }

    #[test]
    fn http_error_without_retry_after_is_still_readable() {
        let e = LyricsError::Http {
            status: 503,
            retry_after_s: None,
        };
        assert_eq!(e.to_string(), "lyrics provider returned 503");
    }
}
