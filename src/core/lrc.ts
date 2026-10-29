/**
 * LRC and Enhanced-LRC parser.
 *
 * Pure and dependency-free so it can be unit-tested exhaustively — lyric sync is
 * only as good as this file. Real-world .lrc files are a mess: BOMs, CRLF, multiple
 * timestamps per line, two- or three-digit fractions, metadata mixed into the body,
 * blank lines used as instrumental gaps. All of that is handled explicitly.
 *
 * Deliberate non-goal: we never synthesise word timings from line timings. A
 * line-timed file is reported as `timingKind: 'line'` so the UI can say so.
 */

import type { LyricLine, LyricWord, LyricsTimingKind, ParsedLyrics } from './types';

/** `[mm:ss.xx]` / `[mm:ss.xxx]` / `[mm:ss]` / `[hh:mm:ss.xx]` at the start of a line. */
const LINE_TIME = /\[(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?\]/g;

/** `<mm:ss.xx>` word markers used by Enhanced LRC. */
const WORD_TIME = /<(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?>/g;

/** `[ti:Title]` — a tag is a bracketed pair whose key is alphabetic. */
const TAG = /^\[([a-zA-Z_]+):(.*)\]$/;

/**
 * Convert a captured time to ms. The fraction is positional, not numeric:
 * `.5` is 500 ms, `.05` is 50 ms, `.005` is 5 ms. Treating it as an integer
 * number of centiseconds is the classic LRC parsing bug.
 */
function toMs(minutes: string, seconds: string, fraction?: string): number {
  const m = Number(minutes);
  const s = Number(seconds);
  let frac = 0;
  if (fraction !== undefined && fraction.length > 0) {
    frac = Number(fraction) / Math.pow(10, fraction.length);
  }
  return Math.round((m * 60 + s + frac) * 1000);
}

function stripBom(input: string): string {
  return input.charCodeAt(0) === 0xfeff ? input.slice(1) : input;
}

/**
 * Parse the `[offset:+/-nnn]` tag. LRC convention is milliseconds, and a
 * *positive* offset means the lyrics should appear earlier, so it is negated
 * here to give "ms to add to a lyric timestamp".
 */
function parseOffsetTag(raw: string | undefined): number {
  if (!raw) return 0;
  const n = Number.parseInt(raw.trim(), 10);
  return Number.isFinite(n) ? -n : 0;
}

function parseWords(body: string): { words: LyricWord[]; plain: string } {
  const words: LyricWord[] = [];
  const matches = [...body.matchAll(WORD_TIME)];
  if (matches.length === 0) {
    return { words: [], plain: body.trim() };
  }

  for (let i = 0; i < matches.length; i += 1) {
    const match = matches[i]!;
    const timeMs = toMs(match[1]!, match[2]!, match[3]);
    const start = match.index! + match[0].length;
    const end = i + 1 < matches.length ? matches[i + 1]!.index! : body.length;
    const text = body.slice(start, end);
    // Keep interior spacing — "don't" and held notes rely on it — but drop
    // fragments that are pure whitespace, which come from `<t1><t2>text`.
    if (text.trim().length > 0) {
      words.push({ timeMs, text });
    }
  }

  const plain = body.replace(WORD_TIME, '').replace(/\s+/g, ' ').trim();
  return { words, plain };
}

export function parseLrc(source: string): ParsedLyrics {
  const tags: Record<string, string> = {};
  const warnings: string[] = [];
  const lines: LyricLine[] = [];
  let sawWordTiming = false;
  let sawLineTiming = false;

  const rawLines = stripBom(source).split(/\r\n|\r|\n/);

  for (let lineNo = 0; lineNo < rawLines.length; lineNo += 1) {
    const raw = rawLines[lineNo]!;
    const trimmed = raw.trim();
    if (trimmed.length === 0) continue;

    const tagMatch = TAG.exec(trimmed);
    if (tagMatch) {
      const key = tagMatch[1]!.toLowerCase();
      // `[offset:...]` has an alphabetic key but is not metadata about the song;
      // it is still stored as a tag and additionally surfaced as fileOffsetMs.
      tags[key] = tagMatch[2]!.trim();
      continue;
    }

    LINE_TIME.lastIndex = 0;
    const stamps = [...trimmed.matchAll(LINE_TIME)];

    if (stamps.length === 0) {
      // Untimed text. Valid in a plain-lyrics file; suspicious inside a timed one.
      lines.push({ timeMs: -1, text: trimmed });
      continue;
    }

    // A timed line may carry several timestamps for a repeated chorus; each
    // produces its own entry sharing the same body.
    const lastStamp = stamps[stamps.length - 1]!;
    const body = trimmed.slice(lastStamp.index! + lastStamp[0].length);

    const { words, plain } = parseWords(body);
    if (words.length > 0) sawWordTiming = true;
    sawLineTiming = true;

    for (const stamp of stamps) {
      const timeMs = toMs(stamp[1]!, stamp[2]!, stamp[3]);
      if (Number(stamp[2]!) >= 60) {
        warnings.push(`line ${lineNo + 1}: seconds field >= 60 ("${stamp[0]}")`);
      }
      const line: LyricLine = { timeMs, text: plain };
      if (words.length > 0) {
        // Re-base word timings for duplicated chorus timestamps: the stored word
        // times belong to the first occurrence, so shift by the stamp delta.
        const delta = timeMs - toMs(stamps[0]![1]!, stamps[0]![2]!, stamps[0]![3]);
        line.words =
          delta === 0 ? words : words.map((w) => ({ ...w, timeMs: w.timeMs + delta }));
      }
      lines.push(line);
    }
  }

  // Untimed lines in an otherwise-timed file are metadata noise or stray text;
  // keep them out of the timeline but record that we dropped them.
  const timed = lines.filter((l) => l.timeMs >= 0);
  const untimedCount = lines.length - timed.length;
  let finalLines: LyricLine[];
  let timingKind: LyricsTimingKind;

  if (sawLineTiming) {
    if (untimedCount > 0) {
      warnings.push(`${untimedCount} untimed line(s) ignored in a timed file`);
    }
    finalLines = timed.sort((a, b) => a.timeMs - b.timeMs);
    timingKind = sawWordTiming ? 'word' : 'line';
  } else {
    finalLines = lines;
    timingKind = 'plain';
  }

  return {
    lines: finalLines,
    timingKind,
    tags,
    fileOffsetMs: parseOffsetTag(tags['offset']),
    warnings,
  };
}

/**
 * Index of the active line at `positionMs`, or -1 before the first line.
 * Binary search: this runs on every animation frame.
 */
export function activeLineIndex(lines: readonly LyricLine[], positionMs: number): number {
  let lo = 0;
  let hi = lines.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (lines[mid]!.timeMs <= positionMs) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return found;
}

/**
 * Index of the active word within a line, or -1 before the first word.
 * Returns -1 for lines without genuine word timings rather than guessing.
 */
export function activeWordIndex(line: LyricLine, positionMs: number): number {
  const words = line.words;
  if (!words || words.length === 0) return -1;
  let found = -1;
  for (let i = 0; i < words.length; i += 1) {
    if (words[i]!.timeMs <= positionMs) found = i;
    else break;
  }
  return found;
}

/**
 * Progress through the active word, 0..1, for partial-word animation.
 *
 * `nextBoundaryMs` is the start of the following word, or the line end. When
 * neither is known the word is treated as instantaneous (returns 1) instead of
 * inventing a duration.
 */
export function wordProgress(
  word: LyricWord,
  nextBoundaryMs: number | undefined,
  positionMs: number,
): number {
  if (nextBoundaryMs === undefined || nextBoundaryMs <= word.timeMs) return 1;
  const t = (positionMs - word.timeMs) / (nextBoundaryMs - word.timeMs);
  return t <= 0 ? 0 : t >= 1 ? 1 : t;
}

/** Gaps longer than this with no lyric line are treated as instrumental. */
export const INSTRUMENTAL_GAP_MS = 5000;

export function isInstrumentalGap(
  lines: readonly LyricLine[],
  positionMs: number,
  gapMs: number = INSTRUMENTAL_GAP_MS,
): boolean {
  const idx = activeLineIndex(lines, positionMs);
  const next = lines[idx + 1];
  if (idx < 0) {
    // Before the first line: an intro counts as instrumental if it is long.
    return next === undefined ? false : next.timeMs - positionMs > gapMs;
  }
  if (next === undefined) return false;
  const current = lines[idx]!;
  return next.timeMs - current.timeMs > gapMs && next.timeMs - positionMs > gapMs;
}

/** Serialise back to LRC, preserving word timings. Used by the editor and export. */
export function toLrc(parsed: ParsedLyrics): string {
  const out: string[] = [];
  for (const [key, value] of Object.entries(parsed.tags)) {
    out.push(`[${key}:${value}]`);
  }
  for (const line of parsed.lines) {
    const stamp = formatStamp(line.timeMs, '[', ']');
    if (line.words && line.words.length > 0) {
      const body = line.words
        .map((w) => `${formatStamp(w.timeMs, '<', '>')}${w.text}`)
        .join('');
      out.push(`${stamp}${body}`);
    } else {
      out.push(`${stamp}${line.text}`);
    }
  }
  return out.join('\n');
}

function formatStamp(ms: number, open: string, close: string): string {
  const total = Math.max(0, ms);
  const minutes = Math.floor(total / 60000);
  const seconds = Math.floor((total % 60000) / 1000);
  const centis = Math.floor((total % 1000) / 10);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${open}${pad(minutes)}:${pad(seconds)}.${pad(centis)}${close}`;
}
