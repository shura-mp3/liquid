/**
 * Live playback state, interpolated at display rate.
 *
 * This is where the native media bridge meets the playback clock built in Phase 0.
 * The bridge emits an observation roughly once a second; the clock turns that into a
 * position that advances smoothly every frame, classifies seeks and pauses, and
 * publishes an uncertainty bound.
 *
 * The clock's `nowMs` and the observation's `observedAtMs` must come from the same
 * monotonic source. They cannot — one is a Rust clock and the other is
 * `performance.now()` — so the native side sends the *age* of its sample instead,
 * and it is converted here. An age survives the crossing between two unrelated
 * clocks; an absolute timestamp does not.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';

import { PlaybackClock, lyricTimeMs } from '../core/clock';
import { makeIdentity, offsetKey } from '../core/identity';
import type { ClockReading, Discontinuity, PlaybackObservation } from '../core/types';
import {
  ipc,
  type MediaStatus,
  type PlaybackEvent,
  type PlaybackSnapshot,
  type SessionSummary,
} from '../providers/ipc';

/**
 * How often the interpolated reading is published to React.
 *
 * Fast enough that a progress bar and a seconds readout look continuous, slow
 * enough that an idle overlay is not re-rendering sixty times a second.
 */
const UPDATE_HZ = 10;

/** Convert a native snapshot into the observation shape the clock consumes. */
export function toObservation(snapshot: PlaybackSnapshot, nowMs: number): PlaybackObservation {
  const identity = makeIdentity({
    provider: 'gsmtc',
    title: snapshot.title,
    ...(snapshot.artist ? { artist: snapshot.artist } : {}),
    ...(snapshot.durationMs !== null ? { durationMs: snapshot.durationMs } : {}),
  });

  return {
    track: identity,
    title: snapshot.title,
    ...(snapshot.artist ? { artist: snapshot.artist } : {}),
    ...(snapshot.album ? { album: snapshot.album } : {}),
    ...(snapshot.durationMs !== null ? { durationMs: snapshot.durationMs } : {}),
    positionMs: snapshot.positionMs,
    status: snapshot.status,
    rate: snapshot.rate,
    // The whole point of the age field: rebase onto this webview's clock.
    observedAtMs: nowMs - snapshot.ageMs,
    timingConfidence: snapshot.timingConfidence,
    controls: snapshot.controls,
    ...(snapshot.sourceApp ? { sourceApp: snapshot.sourceApp } : {}),
    provenance: 'gsmtc',
  };
}

export interface PlaybackView {
  snapshot: PlaybackSnapshot | null;
  sessions: SessionSummary[];
  status: MediaStatus | null;
  /** Interpolated, updated every animation frame while playing. */
  reading: ClockReading | null;
  /** Position with the saved per-track and global offsets applied. */
  lyricTimeMs: number;
  artworkUrl: string | null;
  /** Key for the per-track lyric offset, stable across providers. */
  offsetKey: string | null;
  lastDiscontinuity: Discontinuity | null;
}

export function usePlayback(globalOffsetMs = 0, perTrackOffsetMs = 0): PlaybackView {
  const clock = useRef(new PlaybackClock());
  const [snapshot, setSnapshot] = useState<PlaybackSnapshot | null>(null);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [status, setStatus] = useState<MediaStatus | null>(null);
  const [reading, setReading] = useState<ClockReading | null>(null);
  const [artworkUrl, setArtworkUrl] = useState<string | null>(null);
  const [lastDiscontinuity, setLastDiscontinuity] = useState<Discontinuity | null>(null);
  const artworkKey = useRef<string | null>(null);

  const apply = useCallback((event: PlaybackEvent) => {
    setSessions(event.sessions);
    setSnapshot(event.snapshot);

    if (!event.snapshot) {
      clock.current.reset();
      setReading(null);
      return;
    }
    const kind = clock.current.observe(toObservation(event.snapshot, performance.now()));
    setLastDiscontinuity(kind);
  }, []);

  // Initial state, then live updates.
  useEffect(() => {
    let cancelled = false;
    ipc
      .getMediaStatus()
      .then((s) => {
        if (cancelled) return;
        setStatus(s);
        apply({ snapshot: s.snapshot, sessions: s.sessions });
      })
      .catch((e) => console.error('could not read media status', e));

    const unlisten = listen<PlaybackEvent>('liquid://playback', (e) => apply(e.payload));
    return () => {
      cancelled = true;
      unlisten.then((un) => un()).catch(() => {});
    };
  }, [apply]);

  // Fetch artwork only when the track changes. The native side has already decoded
  // and cached it, so this is a lookup rather than real work.
  useEffect(() => {
    const key = snapshot?.artworkKey ?? null;
    if (key === artworkKey.current) return;
    artworkKey.current = key;

    if (!key) {
      setArtworkUrl(null);
      return;
    }
    let cancelled = false;
    ipc
      .getArtwork(key)
      .then((url) => {
        if (!cancelled) setArtworkUrl(url);
      })
      .catch(() => setArtworkUrl(null));
    return () => {
      cancelled = true;
    };
  }, [snapshot?.artworkKey]);

  /**
   * Publish the interpolated reading while playing, and stop entirely when not.
   *
   * Deliberately **not** once per animation frame. Pushing a new reading into React
   * state at 60 Hz re-renders this whole subtree 60 times a second to move a
   * progress bar a fraction of a pixel and change a millisecond counter nobody is
   * reading that closely — real, permanent CPU cost on a window that is always on
   * top. At `UPDATE_HZ` the progress bar and timings still look smooth.
   *
   * Word-level lyric highlighting in Phase 3 *will* need per-frame precision. That
   * belongs in the lyrics component reading `clock.read()` inside its own rAF loop,
   * so only the text being animated re-renders — not here, where it would drag the
   * entire overlay along with it.
   *
   * A paused position does not move at all, so nothing is scheduled then.
   */
  useEffect(() => {
    const playing = snapshot?.status === 'playing';
    if (!playing) {
      if (clock.current.hasBasis) setReading(clock.current.read(performance.now()));
      return;
    }
    const id = window.setInterval(() => {
      setReading(clock.current.read(performance.now()));
    }, 1000 / UPDATE_HZ);
    return () => window.clearInterval(id);
  }, [snapshot?.status, snapshot?.title]);

  const identity = snapshot
    ? makeIdentity({
        provider: 'gsmtc',
        title: snapshot.title,
        ...(snapshot.artist ? { artist: snapshot.artist } : {}),
        ...(snapshot.durationMs !== null ? { durationMs: snapshot.durationMs } : {}),
      })
    : null;

  return {
    snapshot,
    sessions,
    status,
    reading,
    lyricTimeMs: reading
      ? lyricTimeMs(reading, { globalMs: globalOffsetMs, perTrackMs: perTrackOffsetMs })
      : 0,
    artworkUrl,
    offsetKey: identity ? offsetKey(identity) : null,
    lastDiscontinuity,
  };
}

/** `mm:ss`, or `--:--` when the value is unknown. */
export function formatTime(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms) || ms < 0) return '--:--';
  const total = Math.floor(ms / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${String(seconds).padStart(2, '0')}`;
}
