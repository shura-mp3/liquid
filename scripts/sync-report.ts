/**
 * Prints measured lyric-sync accuracy against the synthetic ground truth.
 * Run with: npx vite-node scripts/sync-report.ts
 *
 * These are real measurements from the deterministic timeline, not estimates.
 */

import { PlaybackClock } from '../src/core/clock';
import { SyntheticTimeline, type SyntheticEvent } from '../src/core/synthetic';

const FRAME_MS = 16;

function measure(timeline: SyntheticTimeline, durationMs: number) {
  const clock = new PlaybackClock();
  const reports = new Set(timeline.reportTimes(durationMs));
  const errors: number[] = [];
  const naiveErrors: number[] = [];
  let lastReported = 0;

  for (let t = 0; t <= durationMs; t += FRAME_MS) {
    for (const r of reports) {
      if (r <= t) {
        const o = timeline.observe(r);
        clock.observe(o);
        lastReported = o.positionMs;
        reports.delete(r);
      }
    }
    if (!clock.hasBasis) continue;
    const truth = timeline.truePositionMs(t);
    errors.push(Math.abs(clock.read(t).positionMs - truth));
    naiveErrors.push(Math.abs(lastReported - truth));
  }

  const stats = (xs: number[]) => {
    const s = [...xs].sort((a, b) => a - b);
    return {
      mean: s.reduce((a, b) => a + b, 0) / s.length,
      p95: s[Math.floor(s.length * 0.95)] ?? 0,
      max: s[s.length - 1] ?? 0,
    };
  };

  return { interpolated: stats(errors), naive: stats(naiveErrors), samples: errors.length };
}

const scenarios: Array<{ name: string; durationMs: number; config?: Partial<{ reportIntervalMs: number; positionGranularityMs: number; reportJitterMs: number; events: SyntheticEvent[] }> }> = [
  { name: 'steady 60s', durationMs: 60_000 },
  { name: 'steady 4min', durationMs: 240_000 },
  { name: 'forward seek', durationMs: 60_000, config: { events: [{ atMs: 20_000, kind: 'seek', value: 150_000 }] } },
  { name: 'backward seek', durationMs: 60_000, config: { events: [{ atMs: 30_000, kind: 'seek', value: 5_000 }] } },
  {
    name: 'scrubbing x4',
    durationMs: 40_000,
    config: {
      events: [
        { atMs: 10_000, kind: 'seek', value: 60_000 },
        { atMs: 12_000, kind: 'seek', value: 30_000 },
        { atMs: 14_000, kind: 'seek', value: 90_000 },
        { atMs: 16_000, kind: 'seek', value: 45_000 },
      ],
    },
  },
  {
    name: 'pause 30s',
    durationMs: 60_000,
    config: { events: [{ atMs: 10_000, kind: 'pause' }, { atMs: 40_000, kind: 'resume' }] },
  },
  { name: 'rate 1.25x', durationMs: 60_000, config: { events: [{ atMs: 15_000, kind: 'rate', value: 1.25 }] } },
  { name: 'coarse source (5s reports)', durationMs: 60_000, config: { reportIntervalMs: 5_000 } },
  { name: 'heavy jitter (150ms)', durationMs: 60_000, config: { reportJitterMs: 150 } },
];

const pad = (s: string, n: number) => s.padEnd(n);
const num = (n: number) => `${n.toFixed(1)} ms`.padStart(10);

console.log('');
console.log('LIQUID lyric sync accuracy vs synthetic ground truth');
console.log('='.repeat(78));
console.log(
  `${pad('scenario', 28)}${pad('mean', 11)}${pad('p95', 11)}${pad('max', 11)}${pad('naive mean', 12)}`,
);
console.log('-'.repeat(78));

for (const s of scenarios) {
  const r = measure(new SyntheticTimeline(s.config ?? {}), s.durationMs);
  console.log(
    `${pad(s.name, 28)}${num(r.interpolated.mean)} ${num(r.interpolated.p95)} ${num(
      r.interpolated.max,
    )} ${num(r.naive.mean)}`,
  );
}

console.log('-'.repeat(78));
console.log('"naive mean" = showing the last reported position with no interpolation,');
console.log('i.e. the behaviour the playback clock exists to replace.');
console.log('');
