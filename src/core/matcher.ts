/**
 * Lyrics candidate scoring.
 *
 * LRCLIB will happily hand back the studio lyrics for a live recording, or a
 * different song with a similar name. Auto-applying those is worse than showing
 * nothing, so a candidate must clear a confidence bar before it is used without
 * asking. Below the bar it is offered as a suggestion for the user to confirm.
 *
 * LRCLIB's own guidance is that duration should be within +/-2 s of the record; that
 * is treated as a hard gate here, not a hint.
 */

import { hasVersionMarker, normaliseArtist, normaliseTitle } from './identity';

/** Hard duration gate, per LRCLIB docs. */
export const DURATION_TOLERANCE_MS = 2000;

/** A candidate at or above this score may be applied without asking. */
export const AUTO_APPLY_THRESHOLD = 0.82;

/** Below this a candidate is not even offered. */
export const SUGGEST_THRESHOLD = 0.5;

export interface LyricsCandidate {
  id: number | string;
  trackName: string;
  artistName: string;
  albumName?: string;
  /** Seconds, as LRCLIB returns it. */
  duration?: number;
  instrumental?: boolean;
  hasWordSync?: boolean;
  syncedLyrics?: string | null;
  plainLyrics?: string | null;
}

export interface MatchQuery {
  title: string;
  artist?: string;
  album?: string;
  durationMs?: number;
}

export type MatchVerdict = 'auto' | 'suggest' | 'reject';

export interface MatchResult {
  candidate: LyricsCandidate;
  score: number;
  verdict: MatchVerdict;
  /** Human-readable reasons, surfaced in the UI so a rejection is explainable. */
  reasons: string[];
}

function clamp01(n: number): number {
  return n < 0 ? 0 : n > 1 ? 1 : n;
}

/** Dice coefficient over character bigrams: robust to word order and small typos. */
export function similarity(a: string, b: string): number {
  if (a === b) return 1;
  if (a.length < 2 || b.length < 2) return a === b ? 1 : 0;

  const bigrams = new Map<string, number>();
  for (let i = 0; i < a.length - 1; i += 1) {
    const g = a.slice(i, i + 2);
    bigrams.set(g, (bigrams.get(g) ?? 0) + 1);
  }

  let hits = 0;
  for (let i = 0; i < b.length - 1; i += 1) {
    const g = b.slice(i, i + 2);
    const count = bigrams.get(g);
    if (count !== undefined && count > 0) {
      bigrams.set(g, count - 1);
      hits += 1;
    }
  }

  return (2 * hits) / (a.length - 1 + (b.length - 1));
}

export function scoreCandidate(query: MatchQuery, candidate: LyricsCandidate): MatchResult {
  const reasons: string[] = [];

  const qTitle = normaliseTitle(query.title);
  const cTitle = normaliseTitle(candidate.trackName);
  const qArtist = normaliseArtist(query.artist);
  const cArtist = normaliseArtist(candidate.artistName);

  // --- hard gates -----------------------------------------------------------

  if (query.durationMs !== undefined && candidate.duration !== undefined) {
    const delta = Math.abs(candidate.duration * 1000 - query.durationMs);
    if (delta > DURATION_TOLERANCE_MS) {
      reasons.push(
        `duration differs by ${Math.round(delta / 1000)}s (limit ${
          DURATION_TOLERANCE_MS / 1000
        }s)`,
      );
      return { candidate, score: 0, verdict: 'reject', reasons };
    }
  } else {
    reasons.push('duration unknown on one side; cannot verify the recording length');
  }

  // A version marker on one side but not the other means a different recording.
  // This is the live/remix/sped-up guard and it is non-negotiable.
  const qMarked = hasVersionMarker(query.title);
  const cMarked = hasVersionMarker(candidate.trackName);
  if (qMarked !== cMarked) {
    reasons.push(
      qMarked
        ? 'this track is marked as an alternate version, the candidate is not'
        : 'the candidate is marked as an alternate version, this track is not',
    );
    return { candidate, score: 0, verdict: 'reject', reasons };
  }

  // --- scoring --------------------------------------------------------------

  const titleScore = similarity(qTitle, cTitle);
  const artistScore = qArtist && cArtist ? similarity(qArtist, cArtist) : 0.5;

  let albumScore = 0.5; // neutral when unknown; most sources omit album
  if (query.album && candidate.albumName) {
    albumScore = similarity(normaliseTitle(query.album), normaliseTitle(candidate.albumName));
  }

  let durationScore = 0.5;
  if (query.durationMs !== undefined && candidate.duration !== undefined) {
    const delta = Math.abs(candidate.duration * 1000 - query.durationMs);
    durationScore = 1 - delta / DURATION_TOLERANCE_MS;
  }

  // Timing quality is a scored component rather than a bonus added after the fact:
  // a perfect metadata match already reaches the weighted ceiling, so an added
  // bonus would be clamped away and could never break a tie.
  let timingScore: number;
  if (candidate.hasWordSync) {
    timingScore = 1;
    reasons.push('has word-level timings');
  } else if (candidate.syncedLyrics) {
    timingScore = 0.55;
    reasons.push('has line-level timings only');
  } else if (candidate.instrumental) {
    timingScore = 0.55;
    reasons.push('marked instrumental');
  } else {
    // Unsynced lyrics cannot drive a karaoke display at all.
    timingScore = 0;
    reasons.push('no timing data (plain lyrics only)');
  }

  const score = clamp01(
    titleScore * 0.42 +
      artistScore * 0.28 +
      durationScore * 0.14 +
      albumScore * 0.09 +
      timingScore * 0.07,
  );

  if (titleScore < 0.6) {
    reasons.push(`title similarity only ${titleScore.toFixed(2)}`);
  }
  if (qArtist && cArtist && artistScore < 0.6) {
    reasons.push(`artist similarity only ${artistScore.toFixed(2)}`);
  }

  const verdict: MatchVerdict =
    score >= AUTO_APPLY_THRESHOLD ? 'auto' : score >= SUGGEST_THRESHOLD ? 'suggest' : 'reject';

  return { candidate, score, verdict, reasons };
}

/** Score every candidate, best first. Rejections are kept so the UI can explain them. */
export function rankCandidates(
  query: MatchQuery,
  candidates: readonly LyricsCandidate[],
): MatchResult[] {
  return candidates
    .map((c) => scoreCandidate(query, c))
    .sort((a, b) => b.score - a.score);
}

/** The single candidate safe to apply with no user confirmation, if any. */
export function pickAutoMatch(
  query: MatchQuery,
  candidates: readonly LyricsCandidate[],
): MatchResult | null {
  const ranked = rankCandidates(query, candidates);
  const best = ranked[0];
  if (!best || best.verdict !== 'auto') return null;

  // Two near-identical top scores mean we cannot tell them apart; ask instead of
  // guessing which recording the user is listening to.
  const second = ranked[1];
  if (second && second.verdict === 'auto' && best.score - second.score < 0.05) {
    return null;
  }
  return best;
}
