/**
 * Synchronised lyrics display.
 *
 * Runs its **own** animation frame loop against the playback clock rather than
 * taking a position through React state. Word highlighting needs per-frame
 * precision, and pushing that through the overlay's shared state would re-render
 * every widget sixty times a second — the cost `usePlayback` deliberately avoids by
 * publishing at 10 Hz. Here the loop writes to one element's class and style, so
 * the expensive part stays local.
 *
 * Two honesty rules are enforced in the markup, not just in comments:
 *   - Word-level highlighting only appears when the file genuinely carried word
 *     timings. A line-timed file is labelled line-timed; no word progress is faked.
 *   - When the clock's uncertainty is too high to trust (a stale or coarse source),
 *     the highlight is visibly de-emphasised instead of animating confidently from
 *     a position we no longer believe.
 */

import { useEffect, useMemo, useRef } from 'react';

import { activeLineIndex, activeWordIndex, isInstrumentalGap, wordProgress } from '../core/lrc';
import { lyricTimeMs, shouldTrustWordSync } from '../core/clock';
import type { PlaybackClockReader } from '../hooks/usePlayback';
import type { ParsedLyrics } from '../core/types';

export type LyricsLayout = 'centered' | 'two-line' | 'scroll' | 'single-word' | 'subtitle';

export const LAYOUTS: Array<{ id: LyricsLayout; label: string }> = [
  { id: 'centered', label: 'Classic centered' },
  { id: 'two-line', label: 'Two-line minimal' },
  { id: 'scroll', label: 'Vertical scrolling' },
  { id: 'single-word', label: 'Huge single word' },
  { id: 'subtitle', label: 'Floating subtitle' },
];

/** How many neighbouring lines each layout shows either side of the active one. */
const CONTEXT: Record<LyricsLayout, { before: number; after: number }> = {
  centered: { before: 1, after: 2 },
  'two-line': { before: 0, after: 1 },
  scroll: { before: 3, after: 4 },
  'single-word': { before: 0, after: 0 },
  subtitle: { before: 0, after: 0 },
};

interface Props {
  lyrics: ParsedLyrics;
  layout: LyricsLayout;
  /** Reads the interpolated clock on demand, without a React round trip. */
  reader: PlaybackClockReader;
  offsetMs: number;
  globalOffsetMs: number;
  attribution: string | null;
}

export function LyricsPanel({
  lyrics,
  layout,
  reader,
  offsetMs,
  globalOffsetMs,
  attribution,
}: Props) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const context = CONTEXT[layout];

  // Line elements are rendered once; the frame loop only toggles their state, so
  // React does no work per frame.
  const lineRefs = useRef<Array<HTMLParagraphElement | null>>([]);
  const lastActive = useRef<number>(-2);

  const wordTimed = lyrics.timingKind === 'word';

  useEffect(() => {
    let frame = 0;

    const tick = () => {
      frame = requestAnimationFrame(tick);
      const reading = reader();
      if (!reading) return;

      const t = lyricTimeMs(reading, {
        perTrackMs: offsetMs,
        globalMs: globalOffsetMs,
        fileMs: lyrics.fileOffsetMs,
      });

      const active = activeLineIndex(lyrics.lines, t);
      const trusted = shouldTrustWordSync(reading);
      const container = containerRef.current;
      if (container) {
        container.dataset['trusted'] = String(trusted);
        container.dataset['instrumental'] = String(isInstrumentalGap(lyrics.lines, t));
      }

      if (active !== lastActive.current) {
        // Line changed: restate every line's role once, not every frame.
        lyrics.lines.forEach((_, i) => {
          const el = lineRefs.current[i];
          if (!el) return;
          el.dataset['state'] = i < active ? 'past' : i === active ? 'active' : 'future';
          const distance = Math.abs(i - active);
          el.dataset['visible'] = String(
            i === active ||
              (i < active && distance <= context.before) ||
              (i > active && distance <= context.after),
          );
        });
        lastActive.current = active;

        if (layout === 'scroll') {
          lineRefs.current[active]?.scrollIntoView({ block: 'center', behavior: 'smooth' });
        }
      }

      // Word highlighting, only where the source actually carried word timings.
      const el = active >= 0 ? lineRefs.current[active] : null;
      if (el && wordTimed) {
        const line = lyrics.lines[active]!;
        const wi = activeWordIndex(line, t);
        const words = el.querySelectorAll<HTMLSpanElement>('[data-word]');
        words.forEach((w, i) => {
          if (i < wi) {
            w.style.setProperty('--lq-word-progress', '1');
          } else if (i === wi) {
            const next = line.words?.[i + 1]?.timeMs ?? lyrics.lines[active + 1]?.timeMs;
            w.style.setProperty(
              '--lq-word-progress',
              wordProgress(line.words![i]!, next, t).toFixed(3),
            );
          } else {
            w.style.setProperty('--lq-word-progress', '0');
          }
        });
      }
    };

    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [lyrics, layout, reader, offsetMs, globalOffsetMs, context.before, context.after, wordTimed]);

  // Reset cached state when the lyric set changes.
  useEffect(() => {
    lastActive.current = -2;
    lineRefs.current = [];
  }, [lyrics]);

  const rendered = useMemo(
    () =>
      lyrics.lines.map((line, i) => (
        <p
          key={`${line.timeMs}-${i}`}
          ref={(el) => {
            lineRefs.current[i] = el;
          }}
          className="lq-ly__line"
          data-state="future"
          data-visible="false"
        >
          {wordTimed && line.words ? (
            line.words.map((w, wi) => (
              <span key={wi} data-word className="lq-ly__word">
                {w.text}
              </span>
            ))
          ) : (
            <span>{line.text}</span>
          )}
        </p>
      )),
    [lyrics, wordTimed],
  );

  return (
    <div
      ref={containerRef}
      className="lq-ly"
      data-layout={layout}
      data-word-timed={wordTimed}
      data-trusted="true"
    >
      <div className="lq-ly__lines">{rendered}</div>

      <footer className="lq-ly__foot">
        {/* The timing kind is stated, never implied. A line-timed file must not
            look like karaoke. */}
        <span className="lq-ly__kind" data-kind={lyrics.timingKind}>
          {lyrics.timingKind === 'word'
            ? 'word-timed'
            : lyrics.timingKind === 'line'
              ? 'line-timed (approximate within a line)'
              : 'untimed'}
        </span>
        {attribution && <span className="lq-ly__credit">{attribution}</span>}
      </footer>
    </div>
  );
}
