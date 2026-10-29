/**
 * The overlay window: a compact HUD capsule.
 *
 * Rebuilt after the fullscreen overlay turned into a desktop-blocking white sheet.
 * The window is now small (420x160 logical by default) and **the window itself is
 * the widget** — there is no canvas of draggable boxes, because a window that size
 * has no room for one, and a window large enough to need one is a window large
 * enough to block the screen.
 *
 * Interaction model:
 *
 * - **Normal:** click-through is on. The capsule shows information and ignores the
 *   mouse entirely, so Spotify and everything behind it stay clickable.
 * - **Edit mode:** click-through off. Dragging the capsule moves the window, and a
 *   Done button restores click-through. Entered from the tray, the settings window,
 *   or Ctrl+Alt+C.
 *
 * Nothing here paints an opaque background. The root is transparent and
 * `pointer-events: none`; only the capsule opts back in, and only in edit mode.
 */

import { useCallback, useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';

import { applyGlassToDom, ipc, type Settings, type TransportAction } from '../providers/ipc';
import { usePlayback, formatTime } from '../hooks/usePlayback';
import { useLyrics } from '../hooks/useLyrics';
import { useAudio } from '../hooks/useAudio';
import { Spectrum, type VisualiserMode } from '../visualizers/Spectrum';
import { activeLineIndex } from '../core/lrc';
import { lyricTimeMs } from '../core/clock';

export function OverlayApp({ label }: { label: string }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [clickThrough, setClickThrough] = useState(true);
  const [visMode] = useState<VisualiserMode>('bars');
  const [currentLine, setCurrentLine] = useState<string | null>(null);

  const playback = usePlayback(settings?.globalOffsetMs ?? 0);
  const lyrics = useLyrics(playback.snapshot, playback.offsetKey);
  const audio = useAudio();

  useEffect(() => {
    let cancelled = false;
    ipc
      .getSettings()
      .then((s) => {
        if (cancelled) return;
        setSettings(s);
        setClickThrough(s.clickThrough);
        applyGlassToDom(s.glass, s.theme);
      })
      .catch((e) => console.error('could not load settings', e));
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const unlisteners = [
      listen<boolean>('liquid://toggle-click-through', (e) => setClickThrough(e.payload)),
      listen<Settings>('liquid://settings-changed', (e) => {
        setSettings(e.payload);
        setClickThrough(e.payload.clickThrough);
        applyGlassToDom(e.payload.glass, e.payload.theme);
      }),
    ];
    return () => {
      unlisteners.forEach((p) => p.then((un) => un()).catch(() => {}));
    };
  }, []);

  // Re-assert topmost when focus moves elsewhere. Event-driven, not polled.
  useEffect(() => {
    const promise = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (!focused) ipc.assertTopmost().catch(() => {});
    });
    return () => {
      promise.then((un) => un()).catch(() => {});
    };
  }, []);

  /**
   * Track the active lyric line in its own loop and publish only the line *text*
   * to React. The clock ticks far faster than the line changes, so this re-renders
   * once per lyric rather than once per frame.
   */
  const lyricsState = lyrics.state;
  const globalOffset = settings?.globalOffsetMs ?? 0;
  const perTrackOffset = lyrics.offsetMs;
  useEffect(() => {
    if (lyricsState.kind !== 'ready') {
      setCurrentLine(null);
      return;
    }
    const parsed = lyricsState.lyrics;
    let frame = 0;
    let lastIndex = -2;

    const tick = () => {
      frame = requestAnimationFrame(tick);
      const reading = playback.reader();
      if (!reading) return;
      const t = lyricTimeMs(reading, {
        perTrackMs: perTrackOffset,
        globalMs: globalOffset,
        fileMs: parsed.fileOffsetMs,
      });
      const index = activeLineIndex(parsed.lines, t);
      if (index === lastIndex) return;
      lastIndex = index;
      setCurrentLine(index >= 0 ? (parsed.lines[index]?.text ?? null) : null);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [lyricsState, playback.reader, globalOffset, perTrackOffset]);

  // Reactive border: straight into CSS variables, no React render per frame.
  useEffect(() => {
    const root = document.documentElement;
    let frame = 0;
    let beat = 0;
    let last = performance.now();
    const tick = (now: number) => {
      frame = requestAnimationFrame(tick);
      const dt = now - last;
      last = now;
      const f = audio.reader();
      if (!f) return;
      beat = f.beat ? 1 : Math.max(0, beat - dt / 320);
      root.style.setProperty('--lq-audio-bass', f.bass.toFixed(3));
      root.style.setProperty('--lq-audio-beat', beat.toFixed(3));
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [audio.reader]);

  const onTransport = useCallback((action: TransportAction) => {
    ipc.mediaTransport(action).catch((e) => console.error(e));
  }, []);

  /** In edit mode, dragging the capsule moves the whole window. */
  const onDragStart = useCallback(
    (event: React.PointerEvent) => {
      if (clickThrough) return;
      if ((event.target as HTMLElement).closest('.lq-no-drag')) return;
      getCurrentWindow()
        .startDragging()
        .catch((e) => console.error(e));
    },
    [clickThrough],
  );

  if (!settings) return null;

  const snapshot = playback.snapshot;
  const editing = !clickThrough;
  const position = playback.reading?.positionMs ?? snapshot?.positionMs ?? 0;

  return (
    <div className="lq-cap-root" data-editing={editing}>
      <div
        className="lq-glass lq-cap"
        data-editing={editing}
        data-reactive="true"
        data-tinted="true"
        onPointerDown={onDragStart}
      >
        {/* Artwork, or a tinted placeholder when the source gives none. */}
        <div className="lq-cap__art" data-empty={!playback.artworkUrl}>
          {playback.artworkUrl ? (
            <img src={playback.artworkUrl} alt="" draggable={false} />
          ) : (
            <span aria-hidden="true" />
          )}
        </div>

        <div className="lq-cap__body">
          {snapshot ? (
            <>
              <p className="lq-cap__title" title={snapshot.title}>
                {snapshot.title}
              </p>
              <p className="lq-cap__sub">
                <span className="lq-cap__artist">
                  {snapshot.artist ?? <span className="lq-muted">no artist reported</span>}
                </span>
                <span className="lq-cap__time">
                  {formatTime(position)}
                  {snapshot.durationMs ? ` / ${formatTime(snapshot.durationMs)}` : ''}
                </span>
              </p>
            </>
          ) : (
            <p className="lq-cap__title lq-muted">
              {playback.status?.bridgeRunning === false
                ? 'Media sessions unavailable'
                : 'Waiting for a supported music source.'}
            </p>
          )}

          {/* One lyric line. Every failure mode explains itself. */}
          <p className="lq-cap__lyric" data-state={lyrics.state.kind}>
            {lyrics.state.kind === 'ready' &&
              (currentLine ?? <span className="lq-muted">♪</span>)}
            {lyrics.state.kind === 'loading' && (
              <span className="lq-muted">Looking for lyrics…</span>
            )}
            {lyrics.state.kind === 'absent' && (
              <span className="lq-muted">Synced lyrics unavailable for this track.</span>
            )}
            {lyrics.state.kind === 'ambiguous' && (
              <span className="lq-muted">
                Several possible lyric matches — choose one in Settings.
              </span>
            )}
            {lyrics.state.kind === 'error' && (
              <span className="lq-warn">{lyrics.state.message}</span>
            )}
            {lyrics.state.kind === 'idle' && !snapshot && (
              <span className="lq-muted">Waiting for a supported music source.</span>
            )}
          </p>
        </div>

        {/* A slim visualiser strip, not a panel. */}
        <div className="lq-cap__vis">
          <Spectrum reader={audio.reader} mode={visMode} fpsCap={settings.fpsCap} height={46} />
        </div>

        {/* Edit chrome, only when click-through is off. */}
        {editing && (
          <div className="lq-cap__edit lq-no-drag">
            <span className="lq-cap__edit-hint">Drag to move · {label}</span>
            <button
              type="button"
              className="lq-btn lq-btn--tiny"
              onClick={() => onTransport('previous')}
              disabled={!snapshot?.controls.previous}
              title={snapshot?.controls.previous ? 'Previous' : 'This source cannot skip back'}
            >
              &lsaquo;&lsaquo;
            </button>
            <button
              type="button"
              className="lq-btn lq-btn--tiny"
              onClick={() => onTransport('play-pause')}
              disabled={!snapshot?.controls.play && !snapshot?.controls.pause}
            >
              {snapshot?.status === 'playing' ? '❚❚' : '▶'}
            </button>
            <button
              type="button"
              className="lq-btn lq-btn--tiny"
              onClick={() => onTransport('next')}
              disabled={!snapshot?.controls.next}
              title={snapshot?.controls.next ? 'Next' : 'This source cannot skip forward'}
            >
              &rsaquo;&rsaquo;
            </button>
            <button
              type="button"
              className="lq-btn lq-btn--tiny"
              onClick={() => ipc.showSettingsWindow().catch(console.error)}
            >
              Settings
            </button>
            <button
              type="button"
              className="lq-btn lq-btn--tiny lq-btn--done"
              onClick={() => {
                ipc.setClickThrough(true).then(setClickThrough).catch(console.error);
              }}
            >
              Done
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
