//! Settings persistence on SQLite, with versioned migrations.
//!
//! Settings are stored as individual key/value rows rather than one serialised blob.
//! A blob means an unknown field added by a newer version is destroyed the moment an
//! older version writes, and it makes a partial read all-or-nothing. Rows migrate
//! and merge cleanly, which is the whole point of not losing a user's setup across
//! upgrades.
//!
//! Migrations are forward-only and recorded in `schema_version`. Anything
//! destructive copies the database file first.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("serialisation error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("setting {key} failed validation: {reason}")]
    Invalid { key: String, reason: String },
}

type Result<T> = std::result::Result<T, SettingsError>;

/// The current schema version. Bump this and add a step in `MIGRATIONS`.
const SCHEMA_VERSION: i64 = 2;

/// Forward-only migrations, applied in order for any version above the stored one.
const MIGRATIONS: &[(i64, &str)] = &[
    (
        1,
        r#"
    CREATE TABLE IF NOT EXISTS user_settings (
        key        TEXT PRIMARY KEY NOT NULL,
        value      TEXT NOT NULL,
        updated_at INTEGER NOT NULL DEFAULT (unixepoch())
    );

    CREATE TABLE IF NOT EXISTS widget_layouts (
        id          INTEGER PRIMARY KEY,
        monitor_id  TEXT NOT NULL,
        widget_kind TEXT NOT NULL,
        x           INTEGER NOT NULL,
        y           INTEGER NOT NULL,
        width       INTEGER NOT NULL,
        height      INTEGER NOT NULL,
        opacity     REAL NOT NULL DEFAULT 1.0,
        locked      INTEGER NOT NULL DEFAULT 0,
        visible     INTEGER NOT NULL DEFAULT 1,
        UNIQUE (monitor_id, widget_kind)
    );

    CREATE INDEX IF NOT EXISTS idx_widget_layouts_monitor
        ON widget_layouts (monitor_id);

    CREATE TABLE IF NOT EXISTS lyrics_offsets (
        offset_key TEXT PRIMARY KEY NOT NULL,
        offset_ms  INTEGER NOT NULL,
        updated_at INTEGER NOT NULL DEFAULT (unixepoch())
    );
    "#,
    ),
    // Phase 3. The DDL lives in the lyrics module so the cache owns its own schema.
    (2, crate::lyrics::cache::SCHEMA_SQL),
];

/// Glass appearance parameters. All continuous so they can be bound to sliders.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GlassSettings {
    pub transparency: f64,
    pub blur: f64,
    pub frost: f64,
    pub tint_strength: f64,
    pub saturation: f64,
    pub edge_glow: f64,
    pub reflection: f64,
    pub border_width: f64,
    pub corner_radius: f64,
    pub ripple_strength: f64,
    pub animation_speed: f64,
    pub shadow_softness: f64,
}

impl Default for GlassSettings {
    fn default() -> Self {
        Self {
            transparency: 0.72,
            blur: 0.65,
            frost: 0.4,
            tint_strength: 0.35,
            saturation: 1.15,
            edge_glow: 0.5,
            reflection: 0.3,
            border_width: 1.0,
            corner_radius: 20.0,
            ripple_strength: 0.45,
            animation_speed: 1.0,
            shadow_softness: 0.6,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub theme: String,
    pub performance_preset: String,
    pub fps_cap: u32,
    pub click_through: bool,
    pub overlay_visible: bool,
    pub launch_at_login: bool,
    pub start_minimised: bool,
    pub global_offset_ms: i64,
    /// See `COMPATIBILITY.md` §4. Default on, with a first-run disclosure, so the
    /// choice is informed rather than hidden.
    pub spotify_audio_reactive_visuals: bool,
    pub history_enabled: bool,
    /// Whether LIQUID may ask LRCLIB for lyrics. On by default because lyrics are
    /// the point of the app and the About panel says where they come from, but it
    /// is a network request carrying the track title and artist, so it is one
    /// switch away.
    pub lyrics_online_lookup: bool,
    /// Capture endpoint id, or `None` to follow the system default.
    pub audio_device_id: Option<String>,
    /// Bands in the spectrum. More is finer but costs a little more per frame.
    pub audio_band_count: usize,
    pub glass: GlassSettings,
    pub hotkeys: std::collections::BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        let mut hotkeys = std::collections::BTreeMap::new();
        hotkeys.insert("toggleLyrics".into(), "Ctrl+Alt+L".into());
        hotkeys.insert("toggleHud".into(), "Ctrl+Alt+M".into());
        hotkeys.insert("toggleGlass".into(), "Ctrl+Alt+G".into());
        hotkeys.insert("toggleClickThrough".into(), "Ctrl+Alt+C".into());
        hotkeys.insert("toggleVisualiser".into(), "Ctrl+Alt+V".into());
        hotkeys.insert("openSettings".into(), "Ctrl+Alt+O".into());

        Self {
            theme: "liquid-midnight".into(),
            performance_preset: "balanced".into(),
            fps_cap: 60,
            click_through: false,
            overlay_visible: true,
            launch_at_login: false,
            start_minimised: false,
            global_offset_ms: 0,
            spotify_audio_reactive_visuals: true,
            history_enabled: true,
            lyrics_online_lookup: true,
            audio_device_id: None,
            audio_band_count: 64,
            glass: GlassSettings::default(),
            hotkeys,
        }
    }
}

/// Reject values that would produce an unusable UI, so a corrupt or hand-edited
/// database cannot leave the app unrenderable.
fn validate(settings: &Settings) -> Result<()> {
    let invalid = |key: &str, reason: String| SettingsError::Invalid {
        key: key.into(),
        reason,
    };

    if !matches!(settings.fps_cap, 30 | 60 | 120 | 144 | 165 | 240) {
        return Err(invalid(
            "fpsCap",
            format!("{} is not a supported cap", settings.fps_cap),
        ));
    }
    if !(0.05..=1.0).contains(&settings.glass.transparency) {
        return Err(invalid(
            "glass.transparency",
            "must be between 0.05 and 1.0; a fully transparent HUD is unusable".into(),
        ));
    }
    if settings.global_offset_ms.abs() > 30_000 {
        return Err(invalid(
            "globalOffsetMs",
            "must be within +/-30000 ms".into(),
        ));
    }
    if !(8..=256).contains(&settings.audio_band_count) {
        return Err(invalid(
            "audioBandCount",
            format!(
                "{} bands is outside the supported 8..256",
                settings.audio_band_count
            ),
        ));
    }
    for (action, combo) in &settings.hotkeys {
        if combo.trim().is_empty() {
            return Err(invalid(
                "hotkeys",
                format!("{action} has an empty key combination"),
            ));
        }
    }
    Ok(())
}

pub struct SettingsStore {
    conn: Connection,
    path: PathBuf,
}

impl SettingsStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        // WAL survives an abrupt termination far better than the rollback journal,
        // which matters for an app the user will kill from the tray.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let mut store = Self {
            conn,
            path: path.to_path_buf(),
        };
        store.migrate()?;
        Ok(store)
    }

    /// The underlying connection, for modules that own their own tables (the
    /// lyrics cache). Guarded by the same mutex that guards this store, so callers
    /// already hold exclusive access.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    fn current_version(&self) -> Result<i64> {
        let v: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        Ok(v)
    }

    fn migrate(&mut self) -> Result<()> {
        let from = self.current_version()?;
        if from >= SCHEMA_VERSION {
            return Ok(());
        }

        // Back up before touching an existing populated database. A fresh database
        // (version 0, no file content) has nothing worth preserving.
        if from > 0 {
            let backup = self.path.with_extension(format!("v{from}.bak"));
            std::fs::copy(&self.path, &backup)?;
            tracing::info!(
                ?backup,
                from,
                to = SCHEMA_VERSION,
                "backed up before migration"
            );
        }

        for (version, sql) in MIGRATIONS {
            if *version > from {
                let tx = self.conn.transaction()?;
                tx.execute_batch(sql)?;
                tx.pragma_update(None, "user_version", version)?;
                tx.commit()?;
                tracing::info!(version, "applied migration");
            }
        }
        Ok(())
    }

    /// Load settings, falling back to the default for any key that is missing or
    /// unreadable. One corrupt row must not prevent the app from starting.
    pub fn load(&self) -> Settings {
        let mut settings = Settings::default();
        let Ok(mut stmt) = self.conn.prepare("SELECT key, value FROM user_settings") else {
            return settings;
        };
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        });
        let Ok(rows) = rows else { return settings };

        let mut merged = serde_json::Map::new();
        for row in rows.flatten() {
            match serde_json::from_str::<serde_json::Value>(&row.1) {
                Ok(value) => {
                    merged.insert(row.0, value);
                }
                Err(e) => tracing::warn!(key = %row.0, error = %e, "skipping unreadable setting"),
            }
        }

        // Merge over the defaults so an unknown or absent key keeps its default
        // instead of failing the whole deserialisation.
        if let Ok(serde_json::Value::Object(mut base)) = serde_json::to_value(&settings) {
            for (k, v) in merged {
                base.insert(k, v);
            }
            match serde_json::from_value::<Settings>(serde_json::Value::Object(base)) {
                Ok(loaded) => {
                    if validate(&loaded).is_ok() {
                        settings = loaded;
                    } else {
                        tracing::warn!("stored settings failed validation; using defaults");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "stored settings unreadable; using defaults"),
            }
        }
        settings
    }

    pub fn save(&self, settings: &Settings) -> Result<()> {
        validate(settings)?;
        let serde_json::Value::Object(map) = serde_json::to_value(settings)? else {
            return Ok(());
        };
        let tx = self.conn.unchecked_transaction()?;
        for (key, value) in map {
            tx.execute(
                "INSERT INTO user_settings (key, value, updated_at)
                 VALUES (?1, ?2, unixepoch())
                 ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = unixepoch()",
                params![key, value.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn reset(&self) -> Result<()> {
        self.conn.execute("DELETE FROM user_settings", [])?;
        Ok(())
    }

    /// Per-track lyric offset storage.
    pub fn set_lyrics_offset(&self, offset_key: &str, offset_ms: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO lyrics_offsets (offset_key, offset_ms, updated_at)
             VALUES (?1, ?2, unixepoch())
             ON CONFLICT(offset_key) DO UPDATE SET offset_ms = ?2, updated_at = unixepoch()",
            params![offset_key, offset_ms],
        )?;
        Ok(())
    }

    pub fn lyrics_offset(&self, offset_key: &str) -> Option<i64> {
        self.conn
            .query_row(
                "SELECT offset_ms FROM lyrics_offsets WHERE offset_key = ?1",
                params![offset_key],
                |row| row.get(0),
            )
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("liquid-test-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn creates_schema_and_round_trips_settings() {
        let path = temp_db("roundtrip");
        let store = SettingsStore::open(&path).unwrap();
        assert_eq!(store.current_version().unwrap(), SCHEMA_VERSION);

        let mut s = Settings {
            theme: "liquid-crimson".into(),
            fps_cap: 120,
            global_offset_ms: -250,
            ..Default::default()
        };
        s.glass.blur = 0.9;
        store.save(&s).unwrap();

        let loaded = store.load();
        assert_eq!(loaded.theme, "liquid-crimson");
        assert_eq!(loaded.fps_cap, 120);
        assert_eq!(loaded.glass.blur, 0.9);
        assert_eq!(loaded.global_offset_ms, -250);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn settings_survive_reopening() {
        let path = temp_db("reopen");
        {
            let store = SettingsStore::open(&path).unwrap();
            let s = Settings {
                theme: "liquid-aurora".into(),
                ..Default::default()
            };
            store.save(&s).unwrap();
        }
        let store = SettingsStore::open(&path).unwrap();
        assert_eq!(store.load().theme, "liquid-aurora");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn migrations_are_listed_in_ascending_order() {
        // They are applied in array order, so a list out of order would run a later
        // migration first and then leave user_version wrong. Cheap to assert.
        let versions: Vec<i64> = MIGRATIONS.iter().map(|(v, _)| *v).collect();
        let mut sorted = versions.clone();
        sorted.sort_unstable();
        assert_eq!(versions, sorted, "MIGRATIONS must be in ascending order");
        assert_eq!(
            versions.last().copied(),
            Some(SCHEMA_VERSION),
            "SCHEMA_VERSION must match the last migration"
        );
    }

    #[test]
    fn the_lyrics_cache_table_exists_after_migration() {
        let path = temp_db("lyricscache");
        let store = SettingsStore::open(&path).unwrap();
        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='lyrics_cache'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "migration v2 did not create lyrics_cache");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn migration_is_idempotent() {
        let path = temp_db("idempotent");
        SettingsStore::open(&path).unwrap();
        let store = SettingsStore::open(&path).unwrap();
        assert_eq!(store.current_version().unwrap(), SCHEMA_VERSION);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rejects_an_unusable_fps_cap() {
        let s = Settings {
            fps_cap: 7,
            ..Default::default()
        };
        assert!(validate(&s).is_err());
    }

    #[test]
    fn rejects_a_fully_transparent_hud() {
        let mut s = Settings::default();
        s.glass.transparency = 0.0;
        assert!(validate(&s).is_err());
    }

    #[test]
    fn rejects_an_absurd_global_offset() {
        let s = Settings {
            global_offset_ms: 10_000_000,
            ..Default::default()
        };
        assert!(validate(&s).is_err());
    }

    #[test]
    fn rejects_an_empty_hotkey() {
        let mut s = Settings::default();
        s.hotkeys.insert("toggleLyrics".into(), "  ".into());
        assert!(validate(&s).is_err());
    }

    #[test]
    fn an_unreadable_row_does_not_prevent_loading() {
        let path = temp_db("corrupt");
        let store = SettingsStore::open(&path).unwrap();
        store.save(&Settings::default()).unwrap();
        store
            .conn
            .execute(
                "INSERT OR REPLACE INTO user_settings (key, value) VALUES ('theme', 'not json')",
                [],
            )
            .unwrap();
        // Falls back to the default theme rather than refusing to start.
        assert_eq!(store.load().theme, Settings::default().theme);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_keys_from_a_newer_version_are_preserved_in_storage() {
        let path = temp_db("forward");
        let store = SettingsStore::open(&path).unwrap();
        store.save(&Settings::default()).unwrap();
        store
            .conn
            .execute(
                "INSERT INTO user_settings (key, value) VALUES ('futureFeature', '42')",
                [],
            )
            .unwrap();
        // Saving from this version must not delete the unknown row.
        store.save(&Settings::default()).unwrap();
        let still_there: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM user_settings WHERE key = 'futureFeature'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(still_there, 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn lyrics_offsets_persist_and_update() {
        let path = temp_db("offsets");
        let store = SettingsStore::open(&path).unwrap();
        assert_eq!(store.lyrics_offset("v1|song|artist|40"), None);
        store.set_lyrics_offset("v1|song|artist|40", 120).unwrap();
        assert_eq!(store.lyrics_offset("v1|song|artist|40"), Some(120));
        store.set_lyrics_offset("v1|song|artist|40", -80).unwrap();
        assert_eq!(store.lyrics_offset("v1|song|artist|40"), Some(-80));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reset_restores_defaults() {
        let path = temp_db("reset");
        let store = SettingsStore::open(&path).unwrap();
        let s = Settings {
            theme: "liquid-light".into(),
            ..Default::default()
        };
        store.save(&s).unwrap();
        store.reset().unwrap();
        assert_eq!(store.load().theme, Settings::default().theme);
        let _ = std::fs::remove_file(&path);
    }
}
