//! Local lyric files.
//!
//! Reading a `.lrc` the user points at, and looking for one sitting next to the
//! audio file. Local files take priority over anything fetched: if someone has gone
//! to the trouble of keeping their own timed lyrics, a network result should never
//! quietly override them.

use std::path::{Path, PathBuf};

use super::{LyricsError, LyricsOrigin, ResolvedLyrics, Result};

/// Lyric files are text. Anything this large is not one, and reading it would be a
/// pointless allocation.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

const EXTENSIONS: [&str; 3] = ["lrc", "elrc", "txt"];

/// Read a lyric file the user chose.
///
/// Decodes as UTF-8 and falls back to a lossy decode rather than refusing: plenty of
/// `.lrc` files in the wild are Windows-1252 or Shift-JIS, and showing most of the
/// lyrics beats showing an error. The BOM is handled by the parser downstream.
pub fn read_file(path: &Path) -> Result<ResolvedLyrics> {
    let metadata = std::fs::metadata(path).map_err(|source| LyricsError::Io {
        path: path.display().to_string(),
        source,
    })?;

    if metadata.len() > MAX_FILE_BYTES {
        return Err(LyricsError::Invalid(format!(
            "{} is {} bytes, which is too large to be a lyric file",
            path.display(),
            metadata.len()
        )));
    }

    let bytes = std::fs::read(path).map_err(|source| LyricsError::Io {
        path: path.display().to_string(),
        source,
    })?;

    let text = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                "lyric file is not valid UTF-8; decoding lossily"
            );
            String::from_utf8_lossy(e.as_bytes()).into_owned()
        }
    };

    if text.trim().is_empty() {
        return Err(LyricsError::Invalid(format!(
            "{} contains no text",
            path.display()
        )));
    }

    Ok(ResolvedLyrics {
        text,
        origin: LyricsOrigin::LocalFile,
        instrumental: false,
        provider_id: None,
        attribution: None,
    })
}

/// Look for a lyric file beside an audio file: `song.mp3` -> `song.lrc`.
///
/// Returns the first match in extension order. Case is left to the filesystem, which
/// on Windows is case-insensitive anyway.
pub fn find_beside(audio_path: &Path) -> Option<PathBuf> {
    let stem = audio_path.file_stem()?;
    let dir = audio_path.parent()?;

    for ext in EXTENSIONS {
        // Built by appending rather than `with_extension`, which *replaces* the last
        // extension: for `a.b.c.mp3` the stem is `a.b.c`, and `with_extension("lrc")`
        // would look for `a.b.lrc`. Dotted filenames are common enough in music
        // libraries that this is a real miss, not a curiosity.
        let mut name = stem.to_os_string();
        name.push(".");
        name.push(ext);
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("liquid-lyrics-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(contents).unwrap();
        p
    }

    #[test]
    fn reads_a_utf8_lrc() {
        let dir = temp_dir("utf8");
        let p = write(&dir, "song.lrc", b"[00:01.00]hello");
        let r = read_file(&p).unwrap();
        assert_eq!(r.text, "[00:01.00]hello");
        assert_eq!(r.origin, LyricsOrigin::LocalFile);
        assert!(r.attribution.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decodes_non_utf8_lossily_rather_than_refusing() {
        let dir = temp_dir("latin1");
        // 0xE9 is 'é' in Windows-1252 and invalid as standalone UTF-8.
        let p = write(&dir, "song.lrc", b"[00:01.00]caf\xE9");
        let r = read_file(&p).unwrap();
        assert!(r.text.starts_with("[00:01.00]caf"), "got {:?}", r.text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_an_empty_file() {
        let dir = temp_dir("empty");
        let p = write(&dir, "song.lrc", b"   \n  \n");
        assert!(read_file(&p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reports_a_missing_file_with_its_path() {
        let missing = std::env::temp_dir().join("liquid-definitely-not-here.lrc");
        let err = read_file(&missing).unwrap_err().to_string();
        assert!(err.contains("liquid-definitely-not-here"), "got {err}");
    }

    #[test]
    fn finds_a_sidecar_lrc() {
        let dir = temp_dir("sidecar");
        write(&dir, "track.lrc", b"[00:01.00]x");
        let audio = dir.join("track.mp3");
        assert_eq!(find_beside(&audio), Some(dir.join("track.lrc")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prefers_lrc_over_txt() {
        let dir = temp_dir("priority");
        write(&dir, "track.txt", b"plain");
        write(&dir, "track.lrc", b"[00:01.00]timed");
        let audio = dir.join("track.flac");
        assert_eq!(find_beside(&audio), Some(dir.join("track.lrc")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn returns_none_when_there_is_no_sidecar() {
        let dir = temp_dir("none");
        let audio = dir.join("track.mp3");
        assert_eq!(find_beside(&audio), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn handles_a_filename_with_dots() {
        let dir = temp_dir("dots");
        write(&dir, "a.b.c.lrc", b"[00:01.00]x");
        let audio = dir.join("a.b.c.mp3");
        assert_eq!(find_beside(&audio), Some(dir.join("a.b.c.lrc")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
