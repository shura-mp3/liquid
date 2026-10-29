/**
 * Track identity normalisation.
 *
 * Two jobs, and they pull in opposite directions:
 *
 *  1. Be stable enough that a saved lyric offset still applies when the same song
 *     comes round again, even though GSMTC reports the title slightly differently.
 *  2. Be strict enough that a remix, a live take, or a sped-up upload does NOT
 *     inherit the studio version's lyrics. That is the single worst failure mode
 *     in a lyrics app, so the duration bucket is part of identity, not advisory.
 */

import type { PlaybackObservation, ProviderId, TrackIdentity } from './types';

/** Duration quantisation. 5 s is wide enough for reporting jitter, narrow enough
 *  that a sped-up or extended version lands in a different bucket. */
export const DURATION_BUCKET_MS = 5000;

/** Suffixes that describe the *upload*, not the recording. Safe to drop. */
const NOISE_PATTERNS: RegExp[] = [
  /\(\s*official\s+(music\s+)?video\s*\)/gi,
  /\[\s*official\s+(music\s+)?video\s*\]/gi,
  /\(\s*official\s+audio\s*\)/gi,
  /\[\s*official\s+audio\s*\]/gi,
  /\(\s*lyric\s*s?\s*(video)?\s*\)/gi,
  /\[\s*lyric\s*s?\s*(video)?\s*\]/gi,
  /\(\s*audio\s*\)/gi,
  /\(\s*visuali[sz]er\s*\)/gi,
  /\(\s*hd\s*\)/gi,
  /\(\s*4k\s*\)/gi,
  /\bfull\s+album\b/gi,
];

/**
 * Markers that mean "a different recording". These are NEVER stripped — stripping
 * them is exactly how a live version ends up with studio-timed lyrics.
 */
const VERSION_MARKERS =
  /\b(live|remix|remaster(?:ed)?|acoustic|instrumental|karaoke|cover|demo|edit|mix|version|sped\s*up|slowed|nightcore|reverb|radio\s*edit|extended|8d)\b/i;

export function hasVersionMarker(title: string): boolean {
  return VERSION_MARKERS.test(title);
}

/**
 * Case-fold, strip diacritics, drop upload noise, collapse punctuation.
 * Preserves version markers (see above) and non-Latin scripts, which must survive
 * intact for Japanese/Korean/Chinese titles.
 */
export function normaliseTitle(raw: string): string {
  let s = stripDiacritics(raw);
  for (const pattern of NOISE_PATTERNS) {
    s = s.replace(pattern, ' ');
  }
  s = s.toLowerCase();
  s = dropIntraWordMarks(s);
  // Keep letters (any script), digits and spaces; punctuation varies by source.
  s = s.replace(/[^\p{L}\p{N}\s]/gu, ' ');
  return s.replace(/\s+/g, ' ').trim();
}

/**
 * Strip Latin diacritics without destroying other scripts.
 *
 * NFKD decomposes Hangul syllables into individual jamo, which look identical but
 * compare unequal, so the result must be recomposed with NFC. Composing back is
 * safe: the combining marks we wanted gone have already been removed.
 */
function stripDiacritics(input: string): string {
  return input.normalize('NFKD').replace(/[̀-ͯ]/g, '').normalize('NFC');
}

/**
 * Apostrophes and similar marks sit *inside* a word, so replacing them with a space
 * would split it: "Don't" must become "dont", not "don t". Sources disagree on
 * straight vs curly quotes, so all variants are removed outright.
 */
function dropIntraWordMarks(input: string): string {
  return input.replace(/['‘’ʼ´`]/g, '');
}

/**
 * Artists arrive as "A feat. B", "A, B", "A & B" or "A x B" depending on the
 * source. Collapse to the primary artist so the same song from Spotify and from a
 * browser tab resolves to one identity.
 */
export function normaliseArtist(raw: string | undefined): string {
  if (!raw) return '';
  let s = stripDiacritics(raw).toLowerCase();
  s = s.split(/\s*(?:feat\.?|ft\.?|featuring|with)\s+/i)[0] ?? s;
  s = s.split(/\s*(?:,|&|\sx\s|\sand\s|\/|;|×)\s*/)[0] ?? s;
  s = dropIntraWordMarks(s);
  s = s.replace(/[^\p{L}\p{N}\s]/gu, ' ');
  return s.replace(/\s+/g, ' ').trim();
}

export function durationBucket(durationMs: number | undefined): number {
  if (durationMs === undefined || !Number.isFinite(durationMs) || durationMs <= 0) {
    // Unknown duration gets its own bucket rather than colliding with bucket 0
    // of genuinely short tracks.
    return -1;
  }
  return Math.round(durationMs / DURATION_BUCKET_MS);
}

export function makeIdentity(input: {
  provider: ProviderId;
  providerTrackId?: string;
  title: string;
  artist?: string;
  durationMs?: number;
}): TrackIdentity {
  const identity: TrackIdentity = {
    provider: input.provider,
    normalisedTitle: normaliseTitle(input.title),
    normalisedArtist: normaliseArtist(input.artist),
    durationBucket: durationBucket(input.durationMs),
  };
  if (input.providerTrackId) {
    identity.providerTrackId = input.providerTrackId;
  }
  return identity;
}

export function sameTrack(a: TrackIdentity, b: TrackIdentity): boolean {
  // A provider-native id, when both sides have one, is authoritative.
  if (a.providerTrackId && b.providerTrackId) {
    return a.provider === b.provider && a.providerTrackId === b.providerTrackId;
  }
  return (
    a.provider === b.provider &&
    a.normalisedTitle === b.normalisedTitle &&
    a.normalisedArtist === b.normalisedArtist &&
    a.durationBucket === b.durationBucket
  );
}

/**
 * Stable string key for storage and offset lookup. Format-versioned so a future
 * change to normalisation can migrate instead of silently losing every offset.
 */
export function identityKey(id: TrackIdentity): string {
  return [
    'v1',
    id.provider,
    id.providerTrackId ?? '',
    id.normalisedTitle,
    id.normalisedArtist,
    String(id.durationBucket),
  ].join('|');
}

/**
 * Offsets are keyed without the provider, so an offset saved while listening in
 * Spotify still applies when the same song plays from a local file.
 */
export function offsetKey(id: TrackIdentity): string {
  return ['v1', id.normalisedTitle, id.normalisedArtist, String(id.durationBucket)].join('|');
}

export function observationIdentity(o: PlaybackObservation): TrackIdentity {
  return makeIdentity({
    provider: o.track.provider,
    ...(o.track.providerTrackId ? { providerTrackId: o.track.providerTrackId } : {}),
    title: o.title,
    ...(o.artist ? { artist: o.artist } : {}),
    ...(o.durationMs !== undefined ? { durationMs: o.durationMs } : {}),
  });
}
