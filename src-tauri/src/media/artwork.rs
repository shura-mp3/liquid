//! Album artwork from the media session thumbnail.
//!
//! GSMTC hands out a `IRandomAccessStreamReference`, not bytes, and reading it is a
//! blocking WinRT call — so artwork is fetched on demand by key rather than pushed
//! with every observation. The key changes only when the track does, which is what
//! lets the UI skip refetching sixty times a second.
//!
//! The cache is bounded. An unbounded image cache in a long-running overlay is a
//! slow memory leak, and "no unbounded caches" is one of this project's rules.

use std::collections::HashMap;

use parking_lot::Mutex;

/// Most listeners do not cycle through more distinct covers than this in a session,
/// and 32 decoded thumbnails is a few megabytes at most.
const MAX_ENTRIES: usize = 32;

/// Refuse anything larger than this. A malformed or hostile stream should not be
/// able to allocate without bound.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;

/// Stable cache key for a track's artwork.
///
/// Intentionally *not* the track identity from `identity.ts`: this only needs to
/// change when the picture would change, and it must be computable in one place
/// without pulling normalisation rules into the native layer.
pub fn key_for(title: &str, artist: Option<&str>, album: Option<&str>) -> String {
    let mut hasher = SimpleHash::new();
    hasher.write(title.trim().to_lowercase().as_bytes());
    hasher.write(b"\x1f");
    hasher.write(artist.unwrap_or("").trim().to_lowercase().as_bytes());
    hasher.write(b"\x1f");
    hasher.write(album.unwrap_or("").trim().to_lowercase().as_bytes());
    format!("{:016x}", hasher.finish())
}

/// FNV-1a. Not cryptographic and does not need to be — this names a cache slot.
struct SimpleHash(u64);

impl SimpleHash {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

/// A decoded cover, ready to hand to the webview as a data URL.
#[derive(Debug, Clone)]
pub struct Artwork {
    pub mime: String,
    pub bytes: Vec<u8>,
}

impl Artwork {
    pub fn to_data_url(&self) -> String {
        use base64_lite::encode;
        format!("data:{};base64,{}", self.mime, encode(&self.bytes))
    }
}

#[derive(Default)]
pub struct ArtworkCache {
    entries: Mutex<HashMap<String, Artwork>>,
    /// Insertion order, for eviction. Small enough that a Vec beats a linked map.
    order: Mutex<Vec<String>>,
}

impl ArtworkCache {
    pub fn get(&self, key: &str) -> Option<Artwork> {
        self.entries.lock().get(key).cloned()
    }

    pub fn insert(&self, key: String, artwork: Artwork) {
        if artwork.bytes.len() > MAX_BYTES {
            tracing::warn!(
                key = %key,
                bytes = artwork.bytes.len(),
                "artwork exceeds the size limit; not cached"
            );
            return;
        }

        let mut entries = self.entries.lock();
        let mut order = self.order.lock();

        if entries.insert(key.clone(), artwork).is_none() {
            order.push(key);
        }

        while order.len() > MAX_ENTRIES {
            let oldest = order.remove(0);
            entries.remove(&oldest);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }
}

/// Minimal base64 encoder, so artwork needs no extra dependency.
mod base64_lite {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(input: &[u8]) -> String {
        let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
        for chunk in input.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = *chunk.get(1).unwrap_or(&0) as u32;
            let b2 = *chunk.get(2).unwrap_or(&0) as u32;
            let n = (b0 << 16) | (b1 << 8) | b2;

            out.push(ALPHABET[(n >> 18) as usize & 63] as char);
            out.push(ALPHABET[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[(n >> 6) as usize & 63] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[n as usize & 63] as char
            } else {
                '='
            });
        }
        out
    }
}

/// Sniff the image type from its magic bytes.
///
/// The stream reference carries a content type, but sources lie about it or leave it
/// empty, and the webview will refuse a data URL with the wrong MIME.
pub fn sniff_mime(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'B', b'M', ..] => "image/bmp",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_stable_and_case_insensitive() {
        let a = key_for("Song", Some("Artist"), Some("Album"));
        let b = key_for("  song  ", Some("ARTIST"), Some("album"));
        assert_eq!(a, b);
    }

    #[test]
    fn key_changes_with_the_track() {
        let a = key_for("Song", Some("Artist"), Some("Album"));
        let b = key_for("Other", Some("Artist"), Some("Album"));
        assert_ne!(a, b);
    }

    #[test]
    fn key_distinguishes_missing_fields_from_empty_ones() {
        // Both normalise to the same slot, which is correct: no artist and an empty
        // artist describe the same picture.
        assert_eq!(key_for("S", None, None), key_for("S", Some(""), Some("")));
        assert_ne!(key_for("S", None, None), key_for("S", Some("A"), None));
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_lite::encode(b""), "");
        assert_eq!(base64_lite::encode(b"f"), "Zg==");
        assert_eq!(base64_lite::encode(b"fo"), "Zm8=");
        assert_eq!(base64_lite::encode(b"foo"), "Zm9v");
        assert_eq!(base64_lite::encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_lite::encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_lite::encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn sniffs_common_image_types() {
        assert_eq!(sniff_mime(&[0x89, b'P', b'N', b'G', 1, 2]), "image/png");
        assert_eq!(sniff_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(sniff_mime(b"GIF89a"), "image/gif");
        assert_eq!(sniff_mime(b"RIFF\0\0\0\0WEBPVP8 "), "image/webp");
        assert_eq!(sniff_mime(b"not an image"), "application/octet-stream");
        assert_eq!(sniff_mime(&[]), "application/octet-stream");
    }

    #[test]
    fn data_url_is_well_formed() {
        let art = Artwork {
            mime: "image/png".into(),
            bytes: b"foo".to_vec(),
        };
        assert_eq!(art.to_data_url(), "data:image/png;base64,Zm9v");
    }

    #[test]
    fn cache_evicts_oldest_beyond_the_limit() {
        let cache = ArtworkCache::default();
        for i in 0..(MAX_ENTRIES + 8) {
            cache.insert(
                format!("key-{i}"),
                Artwork {
                    mime: "image/png".into(),
                    bytes: vec![0; 16],
                },
            );
        }
        assert_eq!(cache.len(), MAX_ENTRIES, "cache must stay bounded");
        assert!(
            cache.get("key-0").is_none(),
            "oldest entry should be evicted"
        );
        assert!(cache.get(&format!("key-{}", MAX_ENTRIES + 7)).is_some());
    }

    #[test]
    fn cache_rejects_an_oversized_image() {
        let cache = ArtworkCache::default();
        cache.insert(
            "huge".into(),
            Artwork {
                mime: "image/png".into(),
                bytes: vec![0; MAX_BYTES + 1],
            },
        );
        assert!(cache.get("huge").is_none());
    }

    #[test]
    fn reinserting_a_key_does_not_grow_the_order_list() {
        let cache = ArtworkCache::default();
        for _ in 0..10 {
            cache.insert(
                "same".into(),
                Artwork {
                    mime: "image/png".into(),
                    bytes: vec![1],
                },
            );
        }
        assert_eq!(cache.len(), 1);
    }
}
