import { describe, expect, it } from 'vitest';
import {
  activeLineIndex,
  activeWordIndex,
  isInstrumentalGap,
  parseLrc,
  toLrc,
  wordProgress,
} from '../src/core/lrc';

describe('parseLrc — timestamps', () => {
  it('parses mm:ss.xx as positional fractions, not centiseconds', () => {
    const r = parseLrc('[00:01.5]half\n[00:02.05]fifty\n[00:03.005]five');
    expect(r.lines.map((l) => l.timeMs)).toEqual([1500, 2050, 3005]);
  });

  it('parses bare mm:ss with no fraction', () => {
    const r = parseLrc('[01:30]line');
    expect(r.lines[0]!.timeMs).toBe(90_000);
  });

  it('accepts a colon as the fraction separator', () => {
    const r = parseLrc('[00:10:50]line');
    expect(r.lines[0]!.timeMs).toBe(10_500);
  });

  it('handles minutes beyond 99 for long tracks', () => {
    const r = parseLrc('[100:00.00]line');
    expect(r.lines[0]!.timeMs).toBe(6_000_000);
  });

  it('warns when the seconds field is out of range instead of silently accepting', () => {
    const r = parseLrc('[00:75.00]bad');
    expect(r.warnings.join()).toMatch(/seconds field/);
  });
});

describe('parseLrc — structure', () => {
  it('strips a UTF-8 BOM', () => {
    const r = parseLrc('﻿[ti:Song]\n[00:01.00]hello');
    expect(r.tags['ti']).toBe('Song');
    expect(r.lines).toHaveLength(1);
  });

  it('handles CRLF line endings', () => {
    const r = parseLrc('[00:01.00]one\r\n[00:02.00]two\r\n');
    expect(r.lines).toHaveLength(2);
  });

  it('expands multiple timestamps on one line into separate entries', () => {
    const r = parseLrc('[00:10.00][01:10.00][02:10.00]chorus');
    expect(r.lines).toHaveLength(3);
    expect(r.lines.map((l) => l.timeMs)).toEqual([10_000, 70_000, 130_000]);
    expect(r.lines.every((l) => l.text === 'chorus')).toBe(true);
  });

  it('sorts lines by time even when the file is out of order', () => {
    const r = parseLrc('[00:30.00]c\n[00:10.00]a\n[00:20.00]b');
    expect(r.lines.map((l) => l.text)).toEqual(['a', 'b', 'c']);
  });

  it('collects metadata tags', () => {
    const r = parseLrc('[ti:Title]\n[ar:Artist]\n[al:Album]\n[00:01.00]x');
    expect(r.tags).toMatchObject({ ti: 'Title', ar: 'Artist', al: 'Album' });
  });

  it('keeps the offset tag in the sense the rest of the engine uses', () => {
    // LRC convention is adjustedLineTime = lineTime - offset, i.e. a positive
    // offset shows lyrics earlier. LIQUID applies offsets by adding to the playback
    // position instead, which is the same comparison, so the raw value carries over
    // unchanged. Negating it here would invert every file that sets the tag.
    expect(parseLrc('[offset:+500]\n[00:10.00]x').fileOffsetMs).toBe(500);
    expect(parseLrc('[offset:-250]\n[00:10.00]x').fileOffsetMs).toBe(-250);
    expect(parseLrc('[00:10.00]x').fileOffsetMs).toBe(0);
  });

  it('applies the offset tag in the direction the convention requires', () => {
    // A file asking for lyrics 500 ms earlier must light its line 500 ms sooner.
    const withTag = parseLrc('[offset:+500]\n[00:10.00]line');
    const line = withTag.lines[0]!;
    const atMs = line.timeMs - 300; // 300 ms before the raw timestamp
    expect(activeLineIndex(withTag.lines, atMs)).toBe(-1);
    expect(activeLineIndex(withTag.lines, atMs + withTag.fileOffsetMs)).toBe(0);
  });

  it('reports a plain lyrics file as plain, not as broken', () => {
    const r = parseLrc('just some words\nand more words');
    expect(r.timingKind).toBe('plain');
    expect(r.lines).toHaveLength(2);
  });

  it('drops untimed stragglers from a timed file but records the fact', () => {
    const r = parseLrc('[00:01.00]good\nstray text\n[00:02.00]good');
    expect(r.lines).toHaveLength(2);
    expect(r.warnings.join()).toMatch(/untimed/);
  });

  it('ignores empty lines', () => {
    const r = parseLrc('[00:01.00]a\n\n\n[00:02.00]b');
    expect(r.lines).toHaveLength(2);
  });

  it('survives a file with nothing but tags', () => {
    const r = parseLrc('[ti:Only]\n[ar:Tags]');
    expect(r.lines).toHaveLength(0);
    expect(r.timingKind).toBe('plain');
  });
});

describe('parseLrc — Enhanced LRC word timings', () => {
  const enhanced = '[00:10.00]<00:10.00>Never <00:10.50>gonna <00:11.20>give';

  it('extracts word timings and marks the file as word-timed', () => {
    const r = parseLrc(enhanced);
    expect(r.timingKind).toBe('word');
    const words = r.lines[0]!.words!;
    expect(words.map((w) => w.timeMs)).toEqual([10_000, 10_500, 11_200]);
    expect(words.map((w) => w.text.trim())).toEqual(['Never', 'gonna', 'give']);
  });

  it('builds a clean plain text alongside the words', () => {
    expect(parseLrc(enhanced).lines[0]!.text).toBe('Never gonna give');
  });

  it('does NOT invent word timings for a line-timed file', () => {
    const r = parseLrc('[00:10.00]Never gonna give');
    expect(r.timingKind).toBe('line');
    expect(r.lines[0]!.words).toBeUndefined();
  });

  it('re-bases word timings for a repeated chorus timestamp', () => {
    const r = parseLrc('[00:10.00][00:40.00]<00:10.00>la <00:10.50>la');
    expect(r.lines[0]!.words!.map((w) => w.timeMs)).toEqual([10_000, 10_500]);
    // Second occurrence is 30 s later, so its words must be too.
    expect(r.lines[1]!.words!.map((w) => w.timeMs)).toEqual([40_000, 40_500]);
  });

  it('handles repeated identical words', () => {
    const r = parseLrc('[00:00.00]<00:00.00>na <00:00.30>na <00:00.60>na');
    expect(r.lines[0]!.words).toHaveLength(3);
  });

  it('keeps punctuation attached to its word', () => {
    const r = parseLrc("[00:00.00]<00:00.00>Don't <00:00.50>stop!");
    expect(r.lines[0]!.words!.map((w) => w.text.trim())).toEqual(["Don't", 'stop!']);
  });

  it('skips whitespace-only fragments from adjacent markers', () => {
    const r = parseLrc('[00:00.00]<00:00.00> <00:00.10>word');
    expect(r.lines[0]!.words!.map((w) => w.text.trim())).toEqual(['word']);
  });
});

describe('activeLineIndex', () => {
  const lines = parseLrc('[00:10.00]a\n[00:20.00]b\n[00:30.00]c').lines;

  it('returns -1 before the first line', () => {
    expect(activeLineIndex(lines, 0)).toBe(-1);
    expect(activeLineIndex(lines, 9_999)).toBe(-1);
  });

  it('activates exactly on the boundary', () => {
    expect(activeLineIndex(lines, 10_000)).toBe(0);
    expect(activeLineIndex(lines, 20_000)).toBe(1);
  });

  it('holds the last line past the end', () => {
    expect(activeLineIndex(lines, 10_000_000)).toBe(2);
  });

  it('handles an empty lyric set', () => {
    expect(activeLineIndex([], 5_000)).toBe(-1);
  });
});

describe('activeWordIndex and wordProgress', () => {
  const line = parseLrc('[00:10.00]<00:10.00>one <00:11.00>two').lines[0]!;

  it('tracks the active word', () => {
    expect(activeWordIndex(line, 9_000)).toBe(-1);
    expect(activeWordIndex(line, 10_000)).toBe(0);
    expect(activeWordIndex(line, 10_999)).toBe(0);
    expect(activeWordIndex(line, 11_000)).toBe(1);
  });

  it('returns -1 for a line with no word timings rather than approximating', () => {
    const plain = parseLrc('[00:10.00]one two').lines[0]!;
    expect(activeWordIndex(plain, 10_500)).toBe(-1);
  });

  it('interpolates partial word progress', () => {
    const w = line.words![0]!;
    expect(wordProgress(w, 11_000, 10_000)).toBe(0);
    expect(wordProgress(w, 11_000, 10_500)).toBeCloseTo(0.5, 5);
    expect(wordProgress(w, 11_000, 11_000)).toBe(1);
  });

  it('clamps progress outside the word', () => {
    const w = line.words![0]!;
    expect(wordProgress(w, 11_000, 5_000)).toBe(0);
    expect(wordProgress(w, 11_000, 50_000)).toBe(1);
  });

  it('treats an unknown next boundary as complete instead of inventing a duration', () => {
    const w = line.words![1]!;
    expect(wordProgress(w, undefined, 11_500)).toBe(1);
  });
});

describe('isInstrumentalGap', () => {
  it('detects a long gap between lines', () => {
    const lines = parseLrc('[00:10.00]a\n[00:40.00]b').lines;
    expect(isInstrumentalGap(lines, 15_000)).toBe(true);
    expect(isInstrumentalGap(lines, 38_000)).toBe(false);
  });

  it('treats a long intro as instrumental', () => {
    const lines = parseLrc('[00:30.00]first').lines;
    expect(isInstrumentalGap(lines, 1_000)).toBe(true);
  });

  it('does not flag short gaps', () => {
    const lines = parseLrc('[00:10.00]a\n[00:12.00]b').lines;
    expect(isInstrumentalGap(lines, 11_000)).toBe(false);
  });
});

describe('toLrc round trip', () => {
  it('preserves line timings and text', () => {
    const src = '[ti:Song]\n[00:10.00]one\n[00:20.50]two';
    const again = parseLrc(toLrc(parseLrc(src)));
    expect(again.lines.map((l) => [l.timeMs, l.text])).toEqual([
      [10_000, 'one'],
      [20_500, 'two'],
    ]);
    expect(again.tags['ti']).toBe('Song');
  });

  it('preserves word timings', () => {
    const src = '[00:10.00]<00:10.00>one <00:11.00>two';
    const again = parseLrc(toLrc(parseLrc(src)));
    expect(again.timingKind).toBe('word');
    expect(again.lines[0]!.words!.map((w) => w.timeMs)).toEqual([10_000, 11_000]);
  });
});
