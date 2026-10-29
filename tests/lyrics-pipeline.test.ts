/**
 * End-to-end lyrics pipeline, against a real captured LRCLIB response.
 *
 * `tests/fixtures/lrclib-response.json` was fetched from the live API and trimmed to
 * the fields the pipeline consumes. Using a recorded response rather than a live
 * call keeps the suite deterministic and offline — the provider is a free,
 * rate-limited service and a test suite has no business hammering it — while still
 * exercising the real shape, including its quirks.
 *
 * This is the path a track actually takes: provider record -> matcher -> parser ->
 * clock -> which line is lit.
 */

import { readFileSync } from 'node:fs';
import { fileURLToPath, URL } from 'node:url';
import { describe, expect, it } from 'vitest';

import { parseLrc, activeLineIndex } from '../src/core/lrc';
import { pickAutoMatch, rankCandidates, type LyricsCandidate } from '../src/core/matcher';
import { PlaybackClock, lyricTimeMs } from '../src/core/clock';
import { makeIdentity, offsetKey } from '../src/core/identity';
import type { PlaybackObservation } from '../src/core/types';

const candidates: LyricsCandidate[] = JSON.parse(
  readFileSync(fileURLToPath(new URL('./fixtures/lrclib-response.json', import.meta.url)), 'utf-8'),
);

/** What the Windows media session would report for this track. */
const nowPlaying = {
  title: 'Bohemian Rhapsody',
  artist: 'Queen',
  album: 'A Night at the Opera',
  durationMs: 355_000,
};

describe('the captured fixture is what we think it is', () => {
  it('is a real synced record', () => {
    expect(candidates).toHaveLength(1);
    const c = candidates[0]!;
    expect(c.trackName).toBe('Bohemian Rhapsody');
    expect(c.artistName).toBe('Queen');
    expect(c.syncedLyrics).toBeTruthy();
    expect(c.instrumental).toBe(false);
  });

  it('is line-timed, not word-timed — which is the norm on this provider', () => {
    // Across several live searches no record with word sync could be found at all.
    // If this ever flips, the word-karaoke path finally has real data to test.
    expect(candidates[0]!.hasWordSync).toBe(false);
  });
});

describe('matching the provider record to the playing track', () => {
  it('auto-applies without asking the user', () => {
    const picked = pickAutoMatch(nowPlaying, candidates);
    expect(picked).not.toBeNull();
    expect(picked!.candidate.id).toBe(candidates[0]!.id);
  });

  it('refuses the same record for a live version of the track', () => {
    const live = { ...nowPlaying, title: 'Bohemian Rhapsody (Live Aid)' };
    expect(pickAutoMatch(live, candidates)).toBeNull();
    expect(rankCandidates(live, candidates)[0]!.verdict).toBe('reject');
  });

  it('refuses when the playing track is a different length', () => {
    // A sped-up upload is a different recording, so studio timings would drift.
    const spedUp = { ...nowPlaying, durationMs: 300_000 };
    expect(pickAutoMatch(spedUp, candidates)).toBeNull();
  });

  it('still matches when the source reported no album', () => {
    // Browsers routinely omit it, as Brave did in live testing.
    const noAlbum = { ...nowPlaying, album: undefined };
    expect(pickAutoMatch(noAlbum, candidates)).not.toBeNull();
  });
});

describe('parsing the provider lyrics', () => {
  const parsed = parseLrc(candidates[0]!.syncedLyrics!);

  it('yields a sensible number of timed lines', () => {
    expect(parsed.lines.length).toBeGreaterThan(40);
    expect(parsed.timingKind).toBe('line');
  });

  it('produces strictly ordered, non-negative timestamps', () => {
    let previous = -1;
    for (const line of parsed.lines) {
      expect(line.timeMs).toBeGreaterThanOrEqual(0);
      expect(line.timeMs).toBeGreaterThanOrEqual(previous);
      previous = line.timeMs;
    }
  });

  it('keeps the lyrics inside the track duration', () => {
    const last = parsed.lines[parsed.lines.length - 1]!;
    expect(last.timeMs).toBeLessThanOrEqual(nowPlaying.durationMs);
  });

  it('strips the leading space this provider puts after each timestamp', () => {
    // LRCLIB emits "[00:00.15] Is this..." — a parser that kept the space would
    // show every line indented.
    expect(parsed.lines.every((l) => l.text === l.text.trimStart())).toBe(true);
  });

  it('does not invent word timings for a line-timed source', () => {
    expect(parsed.lines.every((l) => l.words === undefined)).toBe(true);
  });
});

describe('driving the parsed lyrics from the playback clock', () => {
  const parsed = parseLrc(candidates[0]!.syncedLyrics!);

  function observation(positionMs: number, atMs: number): PlaybackObservation {
    return {
      track: makeIdentity({ provider: 'gsmtc', ...nowPlaying }),
      title: nowPlaying.title,
      artist: nowPlaying.artist,
      album: nowPlaying.album,
      durationMs: nowPlaying.durationMs,
      positionMs,
      status: 'playing',
      rate: 1,
      observedAtMs: atMs,
      timingConfidence: 'exact',
      controls: {
        play: true,
        pause: false,
        next: true,
        previous: true,
        seek: true,
        volume: false,
      },
      provenance: 'gsmtc',
    };
  }

  it('lights the right line as playback advances', () => {
    const clock = new PlaybackClock();
    clock.observe(observation(0, 0));

    const third = parsed.lines[2]!;
    // Half a second into the third line.
    const reading = clock.read(third.timeMs + 500);
    expect(activeLineIndex(parsed.lines, reading.positionMs)).toBe(2);
  });

  it('applies a per-track offset without disturbing the clock', () => {
    const clock = new PlaybackClock();
    clock.observe(observation(0, 0));

    const second = parsed.lines[1]!;
    // Sitting 100 ms before the second line: not yet active...
    const atMs = second.timeMs - 100;
    const reading = clock.read(atMs);
    expect(activeLineIndex(parsed.lines, reading.positionMs)).toBe(0);

    // ...but a +200 ms offset shows lyrics earlier, so it should be. The offset is
    // added to the position, so positive means "further into the song".
    const shifted = lyricTimeMs(reading, { perTrackMs: 200 });
    expect(activeLineIndex(parsed.lines, shifted)).toBe(1);

    // The clock itself must be untouched, so changing the offset again is free.
    expect(clock.read(atMs).positionMs).toBeCloseTo(reading.positionMs, 3);
  });

  it('resyncs to the right line after a seek', () => {
    const clock = new PlaybackClock();
    clock.observe(observation(0, 0));

    const target = parsed.lines[20]!;
    const kind = clock.observe(observation(target.timeMs + 50, 1_000));
    expect(kind).toBe('seek');
    expect(activeLineIndex(parsed.lines, clock.read(1_000).positionMs)).toBe(20);
  });

  it('shows nothing before the first line', () => {
    const clock = new PlaybackClock();
    clock.observe(observation(0, 0));
    expect(activeLineIndex(parsed.lines, clock.read(0).positionMs)).toBe(-1);
  });
});

describe('offset keys', () => {
  it('are stable across providers for the same recording', () => {
    const fromGsmtc = makeIdentity({ provider: 'gsmtc', ...nowPlaying });
    const fromExtension = makeIdentity({ provider: 'extension-youtube', ...nowPlaying });
    // An offset saved while listening in one player must apply in another.
    expect(offsetKey(fromGsmtc)).toBe(offsetKey(fromExtension));
  });

  it('differ for a live version, so its offset cannot leak onto the studio cut', () => {
    const studio = makeIdentity({ provider: 'gsmtc', ...nowPlaying });
    const live = makeIdentity({
      provider: 'gsmtc',
      ...nowPlaying,
      title: 'Bohemian Rhapsody (Live Aid)',
    });
    expect(offsetKey(studio)).not.toBe(offsetKey(live));
  });
});
