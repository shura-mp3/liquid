//! Lyrics resolution, orchestrating cache, local files and the provider.
//!
//! Deliberately stops short of *choosing* a candidate. Scoring lives in
//! `src/core/matcher.ts` with the duration gate and the live/remix guard already
//! tested, so this returns the cache answer plus whatever candidates the provider
//! offered, and the frontend decides. The alternative — a second scorer here — is
//! the duplication trap avoided throughout this codebase.

use serde::Serialize;

use super::{
    cache, lrclib::LrclibClient, LyricsCandidate, LyricsError, LyricsOrigin, LyricsQuery,
    ResolvedLyrics, Result,
};
use rusqlite::Connection;

/// The answer to "what have we got for this track".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupResult {
    /// Already resolved — from cache, a local file, or a previous user choice.
    /// When present the frontend uses it and ignores `candidates`.
    pub resolved: Option<ResolvedLyrics>,
    /// True when we have previously checked and this track genuinely has none.
    /// Distinct from "nothing found this time", which is worth retrying.
    pub known_absent: bool,
    /// Unjudged candidates for the frontend's matcher to score.
    pub candidates: Vec<LyricsCandidate>,
    /// Set when the provider could not be reached, so the UI can say why rather
    /// than showing an indistinguishable "no lyrics".
    pub provider_error: Option<String>,
}

impl LookupResult {
    fn cached(lyrics: ResolvedLyrics) -> Self {
        Self {
            resolved: Some(lyrics),
            known_absent: false,
            candidates: Vec::new(),
            provider_error: None,
        }
    }
}

/// Check the cache only. Cheap, synchronous, no network.
pub fn lookup_cached(conn: &Connection, offset_key: &str) -> Result<Option<LookupResult>> {
    match cache::get(conn, offset_key)? {
        Some(cache::CacheHit::Found(lyrics)) => Ok(Some(LookupResult::cached(lyrics))),
        Some(cache::CacheHit::KnownAbsent) => Ok(Some(LookupResult {
            resolved: None,
            known_absent: true,
            candidates: Vec::new(),
            provider_error: None,
        })),
        None => Ok(None),
    }
}

/// Ask the provider. Blocking — must run off the UI thread.
///
/// Tries the exact endpoint first because it is one cheap query the server can
/// answer precisely, and only falls back to search when that misses. A track whose
/// title and artist are both missing is not looked up at all: it would match
/// anything and cost the provider a request for nothing.
pub fn fetch_candidates(
    client: &LrclibClient,
    query: &LyricsQuery,
) -> Result<Vec<LyricsCandidate>> {
    if query.title.trim().is_empty() {
        return Err(LyricsError::Invalid(
            "cannot look up lyrics without a track title".into(),
        ));
    }

    match client.get_exact(query) {
        Ok(Some(exact)) => return Ok(vec![exact]),
        Ok(None) => {}
        // A failed exact lookup is worth retrying as a search rather than giving up;
        // the two endpoints fail independently.
        Err(e) => tracing::debug!(error = %e, "exact lyrics lookup failed; trying search"),
    }

    client.search(query)
}

/// Store the candidate the frontend chose.
pub fn accept(
    conn: &Connection,
    offset_key: &str,
    candidate: &LyricsCandidate,
) -> Result<ResolvedLyrics> {
    // Prefer synced lyrics; fall back to plain so a track still shows words even
    // when nobody has timed them. The frontend labels untimed lyrics as such.
    let text = candidate
        .synced_lyrics
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            candidate
                .plain_lyrics
                .clone()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_default();

    if text.trim().is_empty() && !candidate.instrumental {
        return Err(LyricsError::Invalid(
            "the chosen record contains no lyrics".into(),
        ));
    }

    let resolved = ResolvedLyrics {
        text,
        origin: LyricsOrigin::Lrclib,
        instrumental: candidate.instrumental,
        provider_id: Some(candidate.id),
        attribution: super::attribution_for(LyricsOrigin::Lrclib),
    };

    cache::put(conn, offset_key, &resolved)?;
    Ok(resolved)
}

/// Record that this track has no lyrics, so the provider is not asked again for a
/// week. See the note in `cache.rs` about why a negative result must be stored.
pub fn accept_absent(conn: &Connection, offset_key: &str) -> Result<()> {
    cache::put_absent(conn, offset_key)
}

/// Store lyrics the user supplied themselves, from a file or the editor.
pub fn accept_local(
    conn: &Connection,
    offset_key: &str,
    mut lyrics: ResolvedLyrics,
) -> Result<ResolvedLyrics> {
    // The user's own text is never credited to a provider.
    lyrics.attribution = super::attribution_for(lyrics.origin);
    lyrics.provider_id = None;
    cache::put(conn, offset_key, &lyrics)?;
    Ok(lyrics)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        cache::migrate(&conn).unwrap();
        conn
    }

    fn candidate() -> LyricsCandidate {
        LyricsCandidate {
            id: 7,
            track_name: "Song".into(),
            artist_name: "Artist".into(),
            album_name: Some("Album".into()),
            duration: Some(200.0),
            instrumental: false,
            has_word_sync: false,
            synced_lyrics: Some("[00:01.00]hello".into()),
            plain_lyrics: Some("hello".into()),
        }
    }

    #[test]
    fn a_cold_cache_reports_a_miss() {
        let conn = db();
        assert!(lookup_cached(&conn, "k").unwrap().is_none());
    }

    #[test]
    fn accept_stores_and_is_then_found_in_cache() {
        let conn = db();
        let stored = accept(&conn, "k", &candidate()).unwrap();
        assert_eq!(stored.provider_id, Some(7));
        assert_eq!(stored.text, "[00:01.00]hello");

        let hit = lookup_cached(&conn, "k").unwrap().unwrap();
        assert!(hit.resolved.is_some());
        assert!(!hit.known_absent);
    }

    #[test]
    fn prefers_synced_lyrics_over_plain() {
        let conn = db();
        let stored = accept(&conn, "k", &candidate()).unwrap();
        assert!(stored.text.starts_with("[00:01.00]"), "should keep timings");
    }

    #[test]
    fn falls_back_to_plain_lyrics_when_nothing_is_timed() {
        let conn = db();
        let mut c = candidate();
        c.synced_lyrics = None;
        let stored = accept(&conn, "k", &c).unwrap();
        assert_eq!(stored.text, "hello");
    }

    #[test]
    fn treats_whitespace_only_synced_lyrics_as_absent() {
        let conn = db();
        let mut c = candidate();
        c.synced_lyrics = Some("   \n ".into());
        let stored = accept(&conn, "k", &c).unwrap();
        assert_eq!(stored.text, "hello", "should have fallen through to plain");
    }

    #[test]
    fn refuses_a_record_with_no_lyrics_at_all() {
        let conn = db();
        let mut c = candidate();
        c.synced_lyrics = None;
        c.plain_lyrics = None;
        assert!(accept(&conn, "k", &c).is_err());
    }

    #[test]
    fn an_instrumental_record_with_no_text_is_accepted() {
        let conn = db();
        let mut c = candidate();
        c.synced_lyrics = None;
        c.plain_lyrics = None;
        c.instrumental = true;
        let stored = accept(&conn, "k", &c).unwrap();
        assert!(stored.instrumental);
    }

    #[test]
    fn a_recorded_absence_is_reported_as_known_absent() {
        let conn = db();
        accept_absent(&conn, "k").unwrap();
        let hit = lookup_cached(&conn, "k").unwrap().unwrap();
        assert!(hit.known_absent);
        assert!(hit.resolved.is_none());
    }

    #[test]
    fn local_lyrics_are_not_credited_to_a_provider() {
        let conn = db();
        let local = ResolvedLyrics {
            text: "[00:01.00]mine".into(),
            origin: LyricsOrigin::LocalFile,
            instrumental: false,
            // Deliberately wrong on the way in; accept_local must clear it.
            provider_id: Some(999),
            attribution: Some("Lyrics from LRCLIB".into()),
        };
        let stored = accept_local(&conn, "k", local).unwrap();
        assert!(stored.attribution.is_none());
        assert_eq!(stored.provider_id, None);
    }

    #[test]
    fn a_lookup_without_a_title_is_refused_before_touching_the_network() {
        let client = LrclibClient::new();
        let query = LyricsQuery {
            title: "   ".into(),
            artist: Some("Artist".into()),
            album: None,
            duration_ms: Some(200_000),
        };
        assert!(fetch_candidates(&client, &query).is_err());
    }
}
