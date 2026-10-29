//! Lyrics cache.
//!
//! Two jobs. The obvious one is speed. The one that matters more is that LRCLIB is a
//! free service with published rate limits: re-requesting the same track every time
//! it comes round would be rude, and caching is how LIQUID keeps a single lookup per
//! track rather than one per play.
//!
//! A negative result is cached too, with an expiry. Without that, a track with no
//! lyrics would hit the network on every single play forever — which is exactly the
//! behaviour the rate limits exist to discourage.

use rusqlite::{params, Connection, OptionalExtension};

use super::{LyricsError, LyricsOrigin, Result, ResolvedLyrics};

/// How long to remember that a track had no lyrics. Long enough to stop hammering
/// the provider, short enough that lyrics added later are eventually picked up.
const NEGATIVE_TTL_SECS: i64 = 7 * 24 * 60 * 60;

/// Reject absurd payloads rather than storing them.
const MAX_LYRICS_BYTES: usize = 512 * 1024;

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS lyrics_cache (
            offset_key   TEXT PRIMARY KEY NOT NULL,
            text         TEXT,
            origin       TEXT NOT NULL,
            provider_id  INTEGER,
            instrumental INTEGER NOT NULL DEFAULT 0,
            -- NULL text + found = 0 records a verified absence, so the provider is
            -- not re-queried for every play of a track that simply has no lyrics.
            found        INTEGER NOT NULL DEFAULT 1,
            updated_at   INTEGER NOT NULL DEFAULT (unixepoch())
        );

        CREATE INDEX IF NOT EXISTS idx_lyrics_cache_updated
            ON lyrics_cache (updated_at);
        "#,
    )
}

fn origin_to_str(origin: LyricsOrigin) -> &'static str {
    match origin {
        LyricsOrigin::Cache => "cache",
        LyricsOrigin::Lrclib => "lrclib",
        LyricsOrigin::LocalFile => "local-file",
        LyricsOrigin::UserEdited => "user-edited",
    }
}

fn origin_from_str(s: &str) -> LyricsOrigin {
    match s {
        "lrclib" => LyricsOrigin::Lrclib,
        "local-file" => LyricsOrigin::LocalFile,
        "user-edited" => LyricsOrigin::UserEdited,
        _ => LyricsOrigin::Cache,
    }
}

/// A cache hit. `Found` carries lyrics; `KnownAbsent` means we checked and there are
/// none, which is a real answer and not the same as a miss.
#[derive(Debug, Clone)]
pub enum CacheHit {
    Found(ResolvedLyrics),
    KnownAbsent,
}

pub fn get(conn: &Connection, offset_key: &str) -> Result<Option<CacheHit>> {
    let row = conn
        .query_row(
            "SELECT text, origin, provider_id, instrumental, found, updated_at
             FROM lyrics_cache WHERE offset_key = ?1",
            params![offset_key],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|e| LyricsError::Db(e.to_string()))?;

    let Some((text, origin, provider_id, instrumental, found, updated_at)) = row else {
        return Ok(None);
    };

    if found == 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if now - updated_at > NEGATIVE_TTL_SECS {
            // The absence has expired; treat as a miss so we look again.
            return Ok(None);
        }
        return Ok(Some(CacheHit::KnownAbsent));
    }

    let origin = origin_from_str(&origin);
    Ok(Some(CacheHit::Found(ResolvedLyrics {
        text: text.unwrap_or_default(),
        // A stored network result keeps its real origin so attribution survives a
        // restart; only the fact that it came from disk this time is incidental.
        origin,
        instrumental: instrumental != 0,
        provider_id,
        attribution: super::attribution_for(origin),
    })))
}

pub fn put(conn: &Connection, offset_key: &str, lyrics: &ResolvedLyrics) -> Result<()> {
    if lyrics.text.len() > MAX_LYRICS_BYTES {
        return Err(LyricsError::Invalid(format!(
            "lyrics are {} bytes, over the {MAX_LYRICS_BYTES} byte limit",
            lyrics.text.len()
        )));
    }

    conn.execute(
        "INSERT INTO lyrics_cache (offset_key, text, origin, provider_id, instrumental, found, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, unixepoch())
         ON CONFLICT(offset_key) DO UPDATE SET
            text = ?2, origin = ?3, provider_id = ?4, instrumental = ?5,
            found = 1, updated_at = unixepoch()",
        params![
            offset_key,
            lyrics.text,
            origin_to_str(lyrics.origin),
            lyrics.provider_id,
            i64::from(lyrics.instrumental),
        ],
    )
    .map_err(|e| LyricsError::Db(e.to_string()))?;
    Ok(())
}

/// Record that a track genuinely has no lyrics.
pub fn put_absent(conn: &Connection, offset_key: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO lyrics_cache (offset_key, text, origin, provider_id, instrumental, found, updated_at)
         VALUES (?1, NULL, 'lrclib', NULL, 0, 0, unixepoch())
         ON CONFLICT(offset_key) DO UPDATE SET
            text = NULL, found = 0, updated_at = unixepoch()",
        params![offset_key],
    )
    .map_err(|e| LyricsError::Db(e.to_string()))?;
    Ok(())
}

pub fn forget(conn: &Connection, offset_key: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM lyrics_cache WHERE offset_key = ?1",
        params![offset_key],
    )
    .map_err(|e| LyricsError::Db(e.to_string()))?;
    Ok(())
}

pub fn entry_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM lyrics_cache", [], |r| r.get(0))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn sample() -> ResolvedLyrics {
        ResolvedLyrics {
            text: "[00:01.00]hello".into(),
            origin: LyricsOrigin::Lrclib,
            instrumental: false,
            provider_id: Some(42),
            attribution: Some("Lyrics from LRCLIB".into()),
        }
    }

    #[test]
    fn a_miss_is_none() {
        let conn = db();
        assert!(get(&conn, "v1|nothing|here|40").unwrap().is_none());
    }

    #[test]
    fn round_trips_a_stored_record() {
        let conn = db();
        put(&conn, "k", &sample()).unwrap();
        let hit = get(&conn, "k").unwrap().unwrap();
        match hit {
            CacheHit::Found(l) => {
                assert_eq!(l.text, "[00:01.00]hello");
                assert_eq!(l.provider_id, Some(42));
                assert_eq!(l.origin, LyricsOrigin::Lrclib);
                // Attribution must survive the round trip, not be dropped on reload.
                assert_eq!(l.attribution.as_deref(), Some("Lyrics from LRCLIB"));
            }
            CacheHit::KnownAbsent => panic!("expected a found record"),
        }
    }

    #[test]
    fn overwrites_rather_than_duplicating() {
        let conn = db();
        put(&conn, "k", &sample()).unwrap();
        let mut second = sample();
        second.text = "[00:02.00]updated".into();
        put(&conn, "k", &second).unwrap();
        assert_eq!(entry_count(&conn), 1);
        match get(&conn, "k").unwrap().unwrap() {
            CacheHit::Found(l) => assert_eq!(l.text, "[00:02.00]updated"),
            _ => panic!(),
        }
    }

    #[test]
    fn a_verified_absence_is_distinct_from_a_miss() {
        let conn = db();
        put_absent(&conn, "k").unwrap();
        match get(&conn, "k").unwrap() {
            Some(CacheHit::KnownAbsent) => {}
            other => panic!("expected KnownAbsent, got {other:?}"),
        }
    }

    #[test]
    fn an_expired_absence_becomes_a_miss_so_we_look_again() {
        let conn = db();
        put_absent(&conn, "k").unwrap();
        // Backdate past the TTL.
        conn.execute(
            "UPDATE lyrics_cache SET updated_at = unixepoch() - ?1 WHERE offset_key = 'k'",
            params![NEGATIVE_TTL_SECS + 10],
        )
        .unwrap();
        assert!(get(&conn, "k").unwrap().is_none());
    }

    #[test]
    fn a_fresh_absence_does_not_expire() {
        let conn = db();
        put_absent(&conn, "k").unwrap();
        conn.execute(
            "UPDATE lyrics_cache SET updated_at = unixepoch() - ?1 WHERE offset_key = 'k'",
            params![NEGATIVE_TTL_SECS - 100],
        )
        .unwrap();
        assert!(matches!(get(&conn, "k").unwrap(), Some(CacheHit::KnownAbsent)));
    }

    #[test]
    fn storing_lyrics_replaces_a_previous_absence() {
        let conn = db();
        put_absent(&conn, "k").unwrap();
        put(&conn, "k", &sample()).unwrap();
        assert!(matches!(get(&conn, "k").unwrap(), Some(CacheHit::Found(_))));
    }

    #[test]
    fn rejects_an_oversized_payload() {
        let conn = db();
        let mut huge = sample();
        huge.text = "x".repeat(MAX_LYRICS_BYTES + 1);
        assert!(put(&conn, "k", &huge).is_err());
        assert_eq!(entry_count(&conn), 0);
    }

    #[test]
    fn forget_removes_an_entry() {
        let conn = db();
        put(&conn, "k", &sample()).unwrap();
        forget(&conn, "k").unwrap();
        assert!(get(&conn, "k").unwrap().is_none());
    }

    #[test]
    fn local_and_user_origins_round_trip_without_false_attribution() {
        let conn = db();
        let mut local = sample();
        local.origin = LyricsOrigin::LocalFile;
        local.provider_id = None;
        put(&conn, "k", &local).unwrap();
        match get(&conn, "k").unwrap().unwrap() {
            CacheHit::Found(l) => {
                assert_eq!(l.origin, LyricsOrigin::LocalFile);
                // The user's own file must not be credited to a provider.
                assert!(l.attribution.is_none());
            }
            _ => panic!(),
        }
    }
}
