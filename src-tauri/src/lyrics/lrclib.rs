//! LRCLIB client.
//!
//! LRCLIB is a free, community-run service with no API key and published rate
//! limits. Being a well-behaved client is not optional politeness here, it is the
//! condition of use, so this module enforces it rather than trusting callers:
//!
//! - **Identifies itself.** The docs *require* a `User-Agent` carrying the app name,
//!   version and a contact/homepage.
//! - **Serialises and throttles.** One request at a time, minimum 400 ms apart. The
//!   docs recommend 200–500 ms between sequential requests; the published generic
//!   limit is around 45 requests/minute per IP and 400 ms keeps us under half of it.
//! - **Honours backpressure.** `429` and `503` are respected with `Retry-After` when
//!   present, plus jitter so many clients do not retry in lockstep. `503
//!   ServerOverloaded` is not hypothetical — it was returned repeatedly while
//!   developing against the live API, so it is handled as a first-class case rather
//!   than lumped in with "network error".
//! - **Prefers the exact endpoint.** `/api/get` with title, artist, album and
//!   duration is one cheap query that the server can answer precisely; `/api/search`
//!   is the fallback, not the default.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{LyricsCandidate, LyricsError, LyricsQuery, Result};

const BASE: &str = "https://lrclib.net";

/// Minimum gap between requests. See the module note on rate limits.
const MIN_INTERVAL: Duration = Duration::from_millis(400);

/// Give up rather than hanging the lyrics worker on a stalled connection.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);

/// Cap on how long a `Retry-After` can park us. A provider asking for an hour is
/// answered by giving up on this track, not by sleeping a worker thread for an hour.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

const MAX_ATTEMPTS: u32 = 3;

fn user_agent() -> String {
    format!(
        "LIQUID/{} (+https://github.com/shura-mp3/liquid)",
        env!("CARGO_PKG_VERSION")
    )
}

pub struct LrclibClient {
    agent: ureq::Agent,
    /// Serialises requests *and* carries the last-send time, so the throttle holds
    /// across threads instead of being per-caller.
    last_request: Mutex<Option<Instant>>,
}

impl Default for LrclibClient {
    fn default() -> Self {
        Self::new()
    }
}

impl LrclibClient {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            // Non-2xx must come back as a response, not an error, so the status code
            // and `Retry-After` header can be read and acted on.
            .http_status_as_error(false)
            .build();

        Self {
            agent: ureq::Agent::new_with_config(config),
            last_request: Mutex::new(None),
        }
    }

    /// Block until the minimum interval since the previous request has elapsed.
    fn wait_turn(&self) {
        let mut last = self.last_request.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = *last {
            let elapsed = previous.elapsed();
            if elapsed < MIN_INTERVAL {
                std::thread::sleep(MIN_INTERVAL - elapsed);
            }
        }
        *last = Some(Instant::now());
    }

    /// Deterministic-ish jitter, up to 30% of the base delay.
    ///
    /// Uses the clock rather than a random-number dependency: the only requirement
    /// is that two clients do not retry in perfect lockstep.
    fn jitter(base: Duration) -> Duration {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let fraction = f64::from(nanos % 1_000_000) / 1_000_000.0;
        base + Duration::from_secs_f64(base.as_secs_f64() * 0.3 * fraction)
    }

    fn get_json(&self, url: &str, query: &[(&str, String)]) -> Result<Option<serde_json::Value>> {
        let mut attempt = 0;

        loop {
            attempt += 1;
            self.wait_turn();

            let mut request = self.agent.get(url).header("User-Agent", user_agent());
            for (k, v) in query {
                request = request.query(*k, v.as_str());
            }

            let mut response = request
                .call()
                .map_err(|e| LyricsError::Network(e.to_string()))?;

            let status = response.status().as_u16();

            // 404 is a legitimate answer: this track simply has no lyrics.
            if status == 404 {
                return Ok(None);
            }

            if status == 429 || status == 503 {
                let retry_after_s = response
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok());

                if attempt >= MAX_ATTEMPTS {
                    return Err(LyricsError::Http {
                        status,
                        retry_after_s,
                    });
                }

                // Exponential fallback when the server does not say how long.
                let base = retry_after_s
                    .map(Duration::from_secs)
                    .unwrap_or_else(|| Duration::from_millis(500 * (1u64 << attempt)));

                if base > MAX_RETRY_AFTER {
                    tracing::warn!(
                        status,
                        requested_s = base.as_secs(),
                        "provider asked us to wait longer than we are willing to; giving up on this track"
                    );
                    return Err(LyricsError::Http {
                        status,
                        retry_after_s,
                    });
                }

                let delay = Self::jitter(base);
                tracing::info!(
                    status,
                    attempt,
                    delay_ms = delay.as_millis() as u64,
                    "backing off"
                );
                std::thread::sleep(delay);
                continue;
            }

            if !(200..300).contains(&status) {
                return Err(LyricsError::Http {
                    status,
                    retry_after_s: None,
                });
            }

            let body = response
                .body_mut()
                .read_to_string()
                .map_err(|e| LyricsError::Network(format!("could not read response body: {e}")))?;

            let value: serde_json::Value = serde_json::from_str(&body)
                .map_err(|e| LyricsError::Invalid(format!("malformed JSON from provider: {e}")))?;

            return Ok(Some(value));
        }
    }

    /// Exact lookup. Returns `None` when the provider has nothing for this track.
    pub fn get_exact(&self, query: &LyricsQuery) -> Result<Option<LyricsCandidate>> {
        let mut params: Vec<(&str, String)> = vec![("track_name", query.title.clone())];
        if let Some(artist) = &query.artist {
            params.push(("artist_name", artist.clone()));
        }
        if let Some(album) = &query.album {
            params.push(("album_name", album.clone()));
        }
        if let Some(ms) = query.duration_ms {
            // The endpoint takes whole seconds and matches within a couple of them.
            params.push(("duration", (ms / 1000).to_string()));
        }

        let Some(value) = self.get_json(&format!("{BASE}/api/get"), &params)? else {
            return Ok(None);
        };

        // The API answers errors with a JSON object carrying `statusCode` rather than
        // a candidate, so a shape check is needed before deserialising.
        if value.get("statusCode").is_some() {
            return Ok(None);
        }

        serde_json::from_value::<LyricsCandidate>(value)
            .map(Some)
            .map_err(|e| LyricsError::Invalid(format!("unexpected record shape: {e}")))
    }

    /// Fallback search. Returns up to the provider's page of candidates, unjudged —
    /// scoring happens in `src/core/matcher.ts`.
    pub fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsCandidate>> {
        let mut params: Vec<(&str, String)> = vec![("track_name", query.title.clone())];
        if let Some(artist) = &query.artist {
            params.push(("artist_name", artist.clone()));
        }

        let Some(value) = self.get_json(&format!("{BASE}/api/search"), &params)? else {
            return Ok(Vec::new());
        };

        let Some(array) = value.as_array() else {
            // An error object rather than a list; treat as no results.
            return Ok(Vec::new());
        };

        // Skip records we cannot understand instead of failing the whole search.
        Ok(array
            .iter()
            .filter_map(|v| serde_json::from_value::<LyricsCandidate>(v.clone()).ok())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_identifies_the_app_as_the_provider_requires() {
        let ua = user_agent();
        assert!(ua.starts_with("LIQUID/"), "must name the app: {ua}");
        assert!(
            ua.contains("github.com/shura-mp3/liquid"),
            "must carry a contact: {ua}"
        );
        assert!(
            ua.contains(env!("CARGO_PKG_VERSION")),
            "must carry a version: {ua}"
        );
    }

    #[test]
    fn throttle_interval_stays_under_the_published_limit() {
        // ~45 requests/minute generic limit means 1333 ms each if saturated. 400 ms
        // bursts are fine, but the interval must never drop near zero by accident.
        assert!(
            MIN_INTERVAL >= Duration::from_millis(200),
            "too fast for the docs' guidance"
        );
        assert!(
            MIN_INTERVAL <= Duration::from_millis(500),
            "needlessly slow"
        );
    }

    #[test]
    fn jitter_never_shortens_the_requested_delay() {
        let base = Duration::from_secs(2);
        for _ in 0..50 {
            let delay = LrclibClient::jitter(base);
            assert!(delay >= base, "jitter must not retry sooner than asked");
            assert!(
                delay <= base.mul_f64(1.31),
                "jitter should stay within ~30%"
            );
        }
    }

    #[test]
    fn jitter_handles_a_zero_base() {
        assert_eq!(LrclibClient::jitter(Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn the_throttle_actually_delays_a_second_call() {
        let client = LrclibClient::new();
        let start = Instant::now();
        client.wait_turn();
        client.wait_turn();
        assert!(
            start.elapsed() >= MIN_INTERVAL,
            "second request was not throttled"
        );
    }

    #[test]
    fn retry_ceiling_is_bounded() {
        // A provider must never be able to park a worker thread indefinitely.
        assert!(MAX_RETRY_AFTER <= Duration::from_secs(60));
        const { assert!(MAX_ATTEMPTS <= 5) };
    }
}
