/**
 * Lyric sync accuracy, measured against a deterministic ground truth.
 *
 * The synthetic timeline knows the true playback position at every instant. The
 * clock only sees what a real source would report: quantised to 1 s, jittered, and
 * arriving once a second. The error between them is the number that decides whether
 * this app's headline feature works, so it is asserted, not eyeballed.
 *
 * These thresholds are deliberate commitments. If a change makes sync worse, a test
 * fails rather than a user noticing lyrics drifting.
 */

import { describe, expect, it } from 'vitest';
import { PlaybackClock } from '../src/core/clock';
import { SyntheticTimeline } from '../src/core/synthetic';
import { activeLineIndex, parseLrc } from '../src/core/lrc';

/** Sample the clock every 16 ms, like a 60 Hz render loop. */
const FRAME_MS = 16;

interface Accuracy {
  meanAbsMs: number;
  p95Ms: number;
  maxMs: number;
  samples: number;
}

function measure(timeline: SyntheticTimeline, durationMs: number): Accuracy {
  const clock = new PlaybackClock();
  const reports = new Set(timeline.reportTimes(durationMs));
  const errors: number[] = [];

  for (let t = 0; t <= durationMs; t += FRAME_MS) {
    // Deliver any source reports due at or before this frame.
    for (const r of reports) {
      if (r <= t) {
        clock.observe(timeline.observe(r));
        reports.delete(r);
      }
    }
    if (!clock.hasBasis) continue;

    const predicted = clock.read(t).positionMs;
    const truth = timeline.truePositionMs(t);
    errors.push(Math.abs(predicted - truth));
  }

  errors.sort((a, b) => a - b);
  const sum = errors.reduce((acc, e) => acc + e, 0);
  return {
    meanAbsMs: sum / errors.length,
    p95Ms: errors[Math.floor(errors.length * 0.95)] ?? 0,
    maxMs: errors[errors.length - 1] ?? 0,
    samples: errors.length,
  };
}

describe('sync accuracy — steady playback', () => {
  it('tracks a plain 60 s playthrough within tight bounds', () => {
    const a = measure(new SyntheticTimeline(), 60_000);
    expect(a.samples).toBeGreaterThan(3_000);
    // Source quantises to 1 s and jitters +/-40 ms, so some error is inherent.
    expect(a.meanAbsMs).toBeLessThan(600);
    expect(a.p95Ms).toBeLessThan(1_100);
  });

  it('beats naive use of the raw reported position', () => {
    const timeline = new SyntheticTimeline();
    const interpolated = measure(timeline, 60_000);

    // What a naive implementation does: show the last reported position, no
    // interpolation. This is the behaviour the clock exists to replace.
    const naive: number[] = [];
    let last = 0;
    const reports = new Set(timeline.reportTimes(60_000));
    for (let t = 0; t <= 60_000; t += FRAME_MS) {
      for (const r of reports) {
        if (r <= t) {
          last = timeline.observe(r).positionMs;
          reports.delete(r);
        }
      }
      naive.push(Math.abs(last - timeline.truePositionMs(t)));
    }
    const naiveMean = naive.reduce((a, b) => a + b, 0) / naive.length;

    expect(interpolated.meanAbsMs).toBeLessThan(naiveMean);
  });

  it('holds accuracy over a 4 minute track without accumulating drift', () => {
    const a = measure(new SyntheticTimeline(), 240_000);
    expect(a.meanAbsMs).toBeLessThan(600);
    expect(a.maxMs).toBeLessThan(1_500);
  });
});

describe('sync accuracy — discontinuities', () => {
  it('recovers from a forward seek', () => {
    const timeline = new SyntheticTimeline({
      events: [{ atMs: 20_000, kind: 'seek', value: 150_000 }],
    });
    const a = measure(timeline, 60_000);
    expect(a.meanAbsMs).toBeLessThan(700);
  });

  it('recovers from a backward seek', () => {
    const timeline = new SyntheticTimeline({
      events: [{ atMs: 30_000, kind: 'seek', value: 5_000 }],
    });
    expect(measure(timeline, 60_000).meanAbsMs).toBeLessThan(700);
  });

  it('handles repeated scrubbing', () => {
    const timeline = new SyntheticTimeline({
      events: [
        { atMs: 10_000, kind: 'seek', value: 60_000 },
        { atMs: 12_000, kind: 'seek', value: 30_000 },
        { atMs: 14_000, kind: 'seek', value: 90_000 },
        { atMs: 16_000, kind: 'seek', value: 45_000 },
      ],
    });
    expect(measure(timeline, 40_000).meanAbsMs).toBeLessThan(900);
  });

  it('does not advance across a pause', () => {
    const timeline = new SyntheticTimeline({
      events: [
        { atMs: 10_000, kind: 'pause' },
        { atMs: 40_000, kind: 'resume' },
      ],
    });
    const a = measure(timeline, 60_000);
    expect(a.meanAbsMs).toBeLessThan(700);
    // The critical failure mode: a 30 s pause being integrated as 30 s of playback.
    expect(a.maxMs).toBeLessThan(2_000);
  });

  it('follows a playback-rate change', () => {
    const timeline = new SyntheticTimeline({
      events: [{ atMs: 15_000, kind: 'rate', value: 1.25 }],
    });
    expect(measure(timeline, 60_000).meanAbsMs).toBeLessThan(800);
  });

  it('resets cleanly on a track change', () => {
    const timeline = new SyntheticTimeline({
      events: [{ atMs: 30_000, kind: 'track', title: 'Second Track' }],
    });
    expect(measure(timeline, 60_000).meanAbsMs).toBeLessThan(800);
  });
});

describe('sync accuracy — degraded sources', () => {
  it('survives a coarse source that only reports every 5 s', () => {
    const timeline = new SyntheticTimeline({ reportIntervalMs: 5_000 });
    const a = measure(timeline, 60_000);
    expect(a.meanAbsMs).toBeLessThan(900);
  });

  it('survives very coarse 1 s position granularity with heavy jitter', () => {
    const timeline = new SyntheticTimeline({
      positionGranularityMs: 1_000,
      reportJitterMs: 150,
    });
    expect(measure(timeline, 60_000).meanAbsMs).toBeLessThan(900);
  });

  it('keeps animating through a long reporting gap and flags the uncertainty', () => {
    const timeline = new SyntheticTimeline({ reportIntervalMs: 30_000 });
    const clock = new PlaybackClock();
    clock.observe(timeline.observe(0));

    // 20 s with no word from the source.
    const reading = clock.read(20_000);
    expect(reading.isStale).toBe(true);
    expect(reading.uncertaintyMs).toBeGreaterThan(200);
    // It still advances rather than freezing...
    expect(reading.positionMs).toBeGreaterThan(19_000);
    // ...and is still roughly right, it just admits it might not be.
    expect(Math.abs(reading.positionMs - timeline.truePositionMs(20_000))).toBeLessThan(1_000);
  });
});

describe('lyric line selection against the timeline', () => {
  const lrc = parseLrc(
    [
      '[00:05.00]line one',
      '[00:10.00]line two',
      '[00:15.00]line three',
      '[00:20.00]line four',
    ].join('\n'),
  );

  it('selects the correct line at every frame of a steady playthrough', () => {
    const timeline = new SyntheticTimeline();
    const clock = new PlaybackClock();
    const reports = new Set(timeline.reportTimes(30_000));
    let wrong = 0;
    let total = 0;

    for (let t = 0; t <= 30_000; t += FRAME_MS) {
      for (const r of reports) {
        if (r <= t) {
          clock.observe(timeline.observe(r));
          reports.delete(r);
        }
      }
      if (!clock.hasBasis) continue;

      const predictedLine = activeLineIndex(lrc.lines, clock.read(t).positionMs);
      const trueLine = activeLineIndex(lrc.lines, timeline.truePositionMs(t));
      total += 1;
      if (predictedLine !== trueLine) wrong += 1;
    }

    // Disagreement is only possible within ~1 line boundary window per 5 s line.
    const wrongFraction = wrong / total;
    expect(wrongFraction).toBeLessThan(0.12);
  });
});
