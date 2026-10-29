/**
 * Lyrics resolution and timing for the current track.
 *
 * The native side does I/O; this does the deciding. When a track changes it asks
 * for whatever is cached, and on a miss scores the provider's candidates with the
 * matcher — the one with the ±2 s duration gate and the live/remix guard, so a
 * studio lyric sheet can never be applied to a live take without being asked.
 *
 * A candidate is only applied silently when `pickAutoMatch` is confident *and* the
 * runner-up is clearly worse. Anything less is offered as a suggestion, because
 * showing the wrong lyrics confidently is worse than showing none.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { parseLrc } from '../core/lrc';
import { pickAutoMatch, rankCandidates } from '../core/matcher';
import type { ParsedLyrics } from '../core/types';
import { ipc, type LyricsCandidate, type PlaybackSnapshot, type ResolvedLyrics } from '../providers/ipc';

/** A scored candidate, carrying the original IPC record so it can be accepted. */
export interface LyricsOption {
  candidate: LyricsCandidate;
  score: number;
  reasons: string[];
}

export type LyricsState =
  | { kind: 'idle' }
  | { kind: 'loading' }
  /** Resolved and parsed, ready to display. */
  | { kind: 'ready'; lyrics: ParsedLyrics; resolved: ResolvedLyrics }
  /** Candidates found but none confident enough to apply unasked. */
  | { kind: 'ambiguous'; options: LyricsOption[] }
  /** Checked, and this track genuinely has none. */
  | { kind: 'absent' }
  /** Could not reach the provider. Distinct from "no lyrics". */
  | { kind: 'error'; message: string };

export interface LyricsView {
  state: LyricsState;
  /** Per-track offset in ms, added to the playback position: positive shows
   *  lyrics earlier, negative shows them later. */
  offsetMs: number;
  setOffset: (ms: number) => void;
  nudgeOffset: (deltaMs: number) => void;
  resetOffset: () => void;
  /** Apply one of the ambiguous options, or re-pick after a wrong auto-match. */
  choose: (candidate: LyricsCandidate) => void;
  /** Forget what is cached and look again. */
  retry: () => void;
  offsetKey: string | null;
}

export function useLyrics(snapshot: PlaybackSnapshot | null, offsetKey: string | null): LyricsView {
  const [state, setState] = useState<LyricsState>({ kind: 'idle' });
  const [offsetMs, setOffsetMs] = useState(0);
  /** Guards against a slow lookup for a previous track overwriting the current one. */
  const requestId = useRef(0);

  const query = useMemo(() => {
    if (!snapshot) return null;
    return {
      title: snapshot.title,
      artist: snapshot.artist,
      album: snapshot.album,
      durationMs: snapshot.durationMs,
    };
  }, [snapshot?.title, snapshot?.artist, snapshot?.album, snapshot?.durationMs]);

  /**
   * The matcher treats a missing field as `undefined`; the IPC layer uses `null`
   * because that is what crosses JSON cleanly. Converted here rather than loosening
   * either type, so "absent" keeps one meaning on each side of the boundary.
   */
  const matchQuery = useMemo(() => {
    if (!query) return null;
    return {
      title: query.title,
      ...(query.artist ? { artist: query.artist } : {}),
      ...(query.album ? { album: query.album } : {}),
      ...(query.durationMs !== null ? { durationMs: query.durationMs } : {}),
    };
  }, [query]);

  const applyResolved = useCallback((resolved: ResolvedLyrics) => {
    if (resolved.instrumental) {
      setState({ kind: 'absent' });
      return;
    }
    const parsed = parseLrc(resolved.text);
    if (parsed.lines.length === 0) {
      setState({ kind: 'absent' });
      return;
    }
    setState({ kind: 'ready', lyrics: parsed, resolved });
  }, []);

  const resolve = useCallback(
    async (force = false) => {
      if (!query || !matchQuery || !offsetKey) {
        setState({ kind: 'idle' });
        return;
      }
      const id = ++requestId.current;
      setState({ kind: 'loading' });

      try {
        if (force) await ipc.lyricsForget(offsetKey);

        const result = await ipc.lyricsLookup(query, offsetKey);
        // A newer track started while this was in flight; its result wins.
        if (id !== requestId.current) return;

        if (result.resolved) {
          applyResolved(result.resolved);
          return;
        }
        if (result.knownAbsent) {
          setState({ kind: 'absent' });
          return;
        }
        if (result.providerError) {
          setState({ kind: 'error', message: result.providerError });
          return;
        }
        if (result.candidates.length === 0) {
          // Nothing offered at all. Remember that, so a free rate-limited service
          // is not asked again on every play of this track.
          await ipc.lyricsAcceptAbsent(offsetKey);
          if (id === requestId.current) setState({ kind: 'absent' });
          return;
        }

        // Scoring works on the matcher's own shape; the original records are kept
        // by id so the accepted one is the exact record the provider returned.
        const byId = new Map(result.candidates.map((c) => [c.id, c]));

        const auto = pickAutoMatch(matchQuery, result.candidates);
        if (auto) {
          const original = byId.get(Number(auto.candidate.id));
          if (original) {
            const stored = await ipc.lyricsAccept(offsetKey, original);
            if (id === requestId.current) applyResolved(stored);
            return;
          }
        }

        // Not confident enough. Offer the plausible ones and let the user decide
        // rather than guessing at a recording we cannot identify.
        const ranked: LyricsOption[] = rankCandidates(matchQuery, result.candidates)
          .filter((r) => r.verdict !== 'reject')
          .flatMap((r) => {
            const original = byId.get(Number(r.candidate.id));
            return original
              ? [{ candidate: original, score: r.score, reasons: r.reasons }]
              : [];
          });
        if (id !== requestId.current) return;
        if (ranked.length === 0) {
          await ipc.lyricsAcceptAbsent(offsetKey);
          if (id === requestId.current) setState({ kind: 'absent' });
          return;
        }
        setState({ kind: 'ambiguous', options: ranked.slice(0, 5) });
      } catch (e) {
        if (id === requestId.current) setState({ kind: 'error', message: String(e) });
      }
    },
    [query, matchQuery, offsetKey, applyResolved],
  );

  useEffect(() => {
    void resolve(false);
  }, [resolve]);

  // Offsets are keyed without the provider, so one saved while listening in
  // Spotify still applies when the same track plays from a local file.
  useEffect(() => {
    if (!offsetKey) {
      setOffsetMs(0);
      return;
    }
    let cancelled = false;
    ipc
      .getLyricOffset(offsetKey)
      .then((ms) => {
        if (!cancelled) setOffsetMs(ms);
      })
      .catch(() => setOffsetMs(0));
    return () => {
      cancelled = true;
    };
  }, [offsetKey]);

  const persistOffset = useCallback(
    (ms: number) => {
      const clamped = Math.max(-30_000, Math.min(30_000, Math.round(ms)));
      setOffsetMs(clamped);
      if (offsetKey) ipc.setLyricOffset(offsetKey, clamped).catch((e) => console.error(e));
    },
    [offsetKey],
  );

  const choose = useCallback(
    (candidate: LyricsCandidate) => {
      if (!offsetKey) return;
      ipc
        .lyricsAccept(offsetKey, candidate)
        .then(applyResolved)
        .catch((e) => setState({ kind: 'error', message: String(e) }));
    },
    [offsetKey, applyResolved],
  );

  return {
    state,
    offsetMs,
    setOffset: persistOffset,
    nudgeOffset: (delta) => persistOffset(offsetMs + delta),
    resetOffset: () => persistOffset(0),
    choose,
    retry: () => void resolve(true),
    offsetKey,
  };
}
