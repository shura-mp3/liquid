/**
 * The overlay window.
 *
 * Phase 2 replaces the Phase 1 placeholder panel with real playback state from the
 * Windows media session. The lyrics panel is still explicitly labelled as awaiting
 * its provider — the timing engine is built and tested, but nothing is resolving
 * lyrics for a track yet, and showing invented lines would misrepresent that.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';

import { applyGlassToDom, ipc, type Settings, type TransportAction } from '../providers/ipc';
import { usePlayback } from '../hooks/usePlayback';
import { NowPlaying } from '../widgets/NowPlaying';
import { LyricsPanel, LAYOUTS as LAYOUT_OPTIONS, type LyricsLayout } from '../widgets/LyricsPanel';
import { useLyrics } from '../hooks/useLyrics';
import { useAudio } from '../hooks/useAudio';
import { Spectrum, VISUALISER_MODES, type VisualiserMode } from '../visualizers/Spectrum';

interface WidgetBox {
  x: number;
  y: number;
}

/** Snap a dragged widget to screen edges and to a grid, like a real HUD editor. */
const SNAP_PX = 12;
const GRID_PX = 8;

function snap(value: number, min: number, max: number): number {
  if (Math.abs(value - min) < SNAP_PX) return min;
  if (Math.abs(value - max) < SNAP_PX) return max;
  return Math.round(value / GRID_PX) * GRID_PX;
}

function useDraggableWidget(initial: WidgetBox, locked: boolean) {
  const [pos, setPos] = useState(initial);
  const ref = useRef<HTMLDivElement | null>(null);
  const drag = useRef<{ dx: number; dy: number } | null>(null);

  const onPointerDown = useCallback(
    (event: React.PointerEvent) => {
      if (locked) return;
      if ((event.target as HTMLElement).closest('.lq-no-drag')) return;
      const el = ref.current;
      if (!el) return;
      el.setPointerCapture(event.pointerId);
      drag.current = { dx: event.clientX - pos.x, dy: event.clientY - pos.y };
    },
    [locked, pos.x, pos.y],
  );

  const onPointerMove = useCallback((event: React.PointerEvent) => {
    const d = drag.current;
    const el = ref.current;
    if (!d || !el) return;
    const maxX = window.innerWidth - el.offsetWidth;
    const maxY = window.innerHeight - el.offsetHeight;
    setPos({
      x: Math.min(Math.max(0, snap(event.clientX - d.dx, 0, maxX)), maxX),
      y: Math.min(Math.max(0, snap(event.clientY - d.dy, 0, maxY)), maxY),
    });
  }, []);

  const onPointerUp = useCallback((event: React.PointerEvent) => {
    drag.current = null;
    ref.current?.releasePointerCapture(event.pointerId);
  }, []);

  return { ref, pos, handlers: { onPointerDown, onPointerMove, onPointerUp } };
}

export function OverlayApp({ label }: { label: string }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [clickThrough, setClickThrough] = useState(false);
  const [locked, setLocked] = useState(false);
  const [showChrome, setShowChrome] = useState(true);
  const [actionError, setActionError] = useState<string | null>(null);

  const playback = usePlayback(settings?.globalOffsetMs ?? 0);
  const lyrics = useLyrics(playback.snapshot, playback.offsetKey);
  const [layout, setLayout] = useState<LyricsLayout>('centered');
  const audio = useAudio();
  const [visMode, setVisMode] = useState<VisualiserMode>('bars');
  const [showVisualiser, setShowVisualiser] = useState(true);

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
      listen('liquid://toggle-hud', () => setShowChrome((v) => !v)),
      listen('liquid://toggle-visualiser', () => setShowVisualiser((v) => !v)),
      listen<Settings>('liquid://settings-changed', (e) => {
        setSettings(e.payload);
        applyGlassToDom(e.payload.glass, e.payload.theme);
      }),
    ];
    return () => {
      unlisteners.forEach((p) => p.then((un) => un()).catch(() => {}));
    };
  }, []);

  useEffect(() => {
    const win = getCurrentWindow();
    const promise = win.onFocusChanged(({ payload: focused }) => {
      if (!focused) ipc.assertTopmost().catch(() => {});
    });
    return () => {
      promise.then((un) => un()).catch(() => {});
    };
  }, []);

  const onTransport = useCallback((action: TransportAction) => {
    setActionError(null);
    ipc.mediaTransport(action).catch((e) => setActionError(String(e)));
  }, []);

  const onSeek = useCallback((positionMs: number) => {
    setActionError(null);
    ipc.mediaSeek(positionMs).catch((e) => setActionError(String(e)));
  }, []);

  const nowPlaying = useDraggableWidget({ x: 48, y: 96 }, locked);
  /**
   * Reactive borders: write audio energy straight into CSS custom properties on
   * the overlay root, inside the audio loop. CSS then drives the glow with no
   * React render at all — the alternative, putting bass level into state at 50 Hz,
   * would re-render every widget for a border colour.
   */
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
    return () => {
      cancelAnimationFrame(frame);
      root.style.removeProperty('--lq-audio-bass');
      root.style.removeProperty('--lq-audio-beat');
    };
  }, [audio.reader]);

  const visBox = useDraggableWidget({ x: 440, y: 96 }, locked);
  const lyricsBox = useDraggableWidget({ x: 48, y: 420 }, locked);

  if (!settings) return null;

  const sourceName =
    playback.sessions.find((s) => s.sourceApp === playback.snapshot?.sourceApp)
      ?.displayName ??
    playback.snapshot?.sourceApp ??
    null;


  return (
    <div className="lq-overlay-root" data-chrome={showChrome}>
      {/* Now playing — real data from the Windows media session. */}
      <div
        ref={nowPlaying.ref}
        className="lq-glass lq-widget lq-widget--np"
        data-reactive="true"
        data-interactive={!clickThrough}
        data-tinted="true"
        style={{ transform: `translate3d(${nowPlaying.pos.x}px, ${nowPlaying.pos.y}px, 0)` }}
        {...nowPlaying.handlers}
      >
        <header className="lq-widget__bar">
          <span className="lq-widget__title">Now playing</span>
          <span className="lq-widget__id" title="Which overlay window this is">
            {label}
          </span>
          <span className="lq-pill lq-pill--live">live</span>
        </header>

        <NowPlaying
          snapshot={playback.snapshot}
          reading={playback.reading}
          artworkUrl={playback.artworkUrl}
          sourceName={sourceName}
          onTransport={onTransport}
          onSeek={onSeek}
          bridgeError={playback.status?.bridgeRunning === false ? playback.status.error : null}
        />

        {actionError && <p className="lq-widget__note lq-widget__note--warn">{actionError}</p>}

        <div className="lq-widget__actions lq-no-drag">
          <button
            type="button"
            className="lq-btn lq-btn--ghost"
            onClick={() => {
              ipc.toggleClickThrough().then(setClickThrough).catch(console.error);
            }}
          >
            {clickThrough ? 'Disable' : 'Enable'} click-through
          </button>
          <button type="button" className="lq-btn lq-btn--ghost" onClick={() => setLocked((v) => !v)}>
            {locked ? 'Unlock' : 'Lock'}
          </button>
          <button
            type="button"
            className="lq-btn lq-btn--ghost"
            onClick={() => ipc.showSettingsWindow().catch(console.error)}
          >
            Settings
          </button>
        </div>

        {clickThrough && (
          <p className="lq-widget__note lq-widget__note--warn">
            Click-through is on, so this panel ignores the mouse. Press{' '}
            <kbd>{settings.hotkeys['toggleClickThrough'] ?? 'Ctrl+Alt+C'}</kbd> or use the
            tray icon to get it back.
          </p>
        )}
      </div>

      {/* Visualiser. Driven by real FFT output from WASAPI loopback capture. */}
      {showVisualiser && (
        <div
          ref={visBox.ref}
          className="lq-glass lq-widget lq-widget--vis"
          data-reactive="true"
          data-interactive={!clickThrough}
          data-tinted="true"
          style={{ transform: `translate3d(${visBox.pos.x}px, ${visBox.pos.y}px, 0)` }}
          {...visBox.handlers}
        >
          <header className="lq-widget__bar">
            <span className="lq-widget__title">Visualiser</span>
            <select
              className="lq-input lq-input--compact lq-no-drag"
              value={visMode}
              onChange={(e) => setVisMode(e.target.value as VisualiserMode)}
              aria-label="Visualiser mode"
            >
              {VISUALISER_MODES.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.label}
                </option>
              ))}
            </select>
          </header>

          <Spectrum
            reader={audio.reader}
            mode={visMode}
            fpsCap={settings.fpsCap}
            height={140}
          />

          {/* Capture problems are stated. A flat visualiser with no explanation
              is indistinguishable from a broken one. */}
          {audio.status && audio.status.state !== 'running' && (
            <p className="lq-widget__note lq-widget__note--warn">
              {audio.status.error ?? `Audio capture is ${audio.status.state}.`}
            </p>
          )}
          {audio.status?.state === 'running' && !audio.receiving && (
            <p className="lq-widget__note">Waiting for audio…</p>
          )}
          {audio.status?.state === 'running' && audio.receiving && (
            <p className="lq-widget__note">
              {audio.status.deviceName} · {(audio.status.sampleRate / 1000).toFixed(1)} kHz ·
              system mix, not isolated to one app
            </p>
          )}
        </div>
      )}

      {/* Lyrics. Real now: resolved, matched and timed against the live clock. */}
      <div
        ref={lyricsBox.ref}
        className="lq-glass lq-widget lq-widget--lyrics"
        data-interactive={!clickThrough}
        data-tinted="true"
        style={{ transform: `translate3d(${lyricsBox.pos.x}px, ${lyricsBox.pos.y}px, 0)` }}
        {...lyricsBox.handlers}
      >
        <header className="lq-widget__bar">
          <span className="lq-widget__title">Lyrics</span>
          <select
            className="lq-input lq-input--compact lq-no-drag"
            value={layout}
            onChange={(e) => setLayout(e.target.value as LyricsLayout)}
            aria-label="Lyrics layout"
          >
            {LAYOUT_OPTIONS.map((l) => (
              <option key={l.id} value={l.id}>
                {l.label}
              </option>
            ))}
          </select>
        </header>

        {/* Some sources publish a timeline once and never update it, so the
            reported position is frozen. Syncing to that would confidently show the
            wrong line, so the panel says so instead. */}
        {playback.snapshot?.timingConfidence === 'unknown' && (
          <p className="lq-widget__note lq-widget__note--warn">
            {sourceName ?? 'This source'} is not reporting playback position, so lyrics
            cannot be synchronised to it. Track details still work.
          </p>
        )}

        {lyrics.state.kind === 'ready' && playback.snapshot?.timingConfidence !== 'unknown' && (
          <LyricsPanel
            lyrics={lyrics.state.lyrics}
            layout={layout}
            reader={playback.reader}
            offsetMs={lyrics.offsetMs}
            globalOffsetMs={settings.globalOffsetMs}
            attribution={lyrics.state.resolved.attribution}
          />
        )}

        {lyrics.state.kind === 'loading' && <p className="lq-muted">Looking for lyrics…</p>}
        {lyrics.state.kind === 'idle' && <p className="lq-muted">Waiting for a track.</p>}
        {lyrics.state.kind === 'absent' && (
          <p className="lq-muted">No lyrics found for this track.</p>
        )}
        {lyrics.state.kind === 'error' && (
          <p className="lq-widget__note lq-widget__note--warn">{lyrics.state.message}</p>
        )}

        {/* Not confident enough to apply one unasked, so the user picks. */}
        {lyrics.state.kind === 'ambiguous' && (
          <div className="lq-ly__choices lq-no-drag">
            <p className="lq-muted">
              Several possible matches — none certain enough to apply automatically.
            </p>
            {lyrics.state.options.map((o) => (
              <button
                key={o.candidate.id}
                type="button"
                className="lq-ly__choice"
                onClick={() => lyrics.choose(o.candidate)}
                title={o.reasons.join(' · ')}
              >
                <span className="lq-ly__choice-name">
                  {o.candidate.artistName} — {o.candidate.trackName}
                </span>
                <span className="lq-ly__choice-score">{Math.round(o.score * 100)}%</span>
              </button>
            ))}
          </div>
        )}

        {/* Offset controls. Saved per track, keyed without the provider so an
            adjustment made in one player still applies in another. */}
        {playback.offsetKey && (
          <div className="lq-ly__offset lq-no-drag">
            <span className="lq-ly__offset-label">
              Offset <em>{lyrics.offsetMs > 0 ? `+${lyrics.offsetMs}` : lyrics.offsetMs} ms</em>
            </span>
            <div className="lq-ly__offset-buttons">
              {[-100, -50, -10, 10, 50, 100].map((d) => (
                <button
                  key={d}
                  type="button"
                  className="lq-btn lq-btn--ghost lq-btn--tiny"
                  onClick={() => lyrics.nudgeOffset(d)}
                  title={d > 0 ? 'Show lyrics earlier' : 'Show lyrics later'}
                >
                  {d > 0 ? `+${d}` : d}
                </button>
              ))}
              <button
                type="button"
                className="lq-btn lq-btn--ghost lq-btn--tiny"
                onClick={lyrics.resetOffset}
              >
                reset
              </button>
              <button
                type="button"
                className="lq-btn lq-btn--ghost lq-btn--tiny"
                onClick={lyrics.retry}
                title="Forget the cached match and look again"
              >
                re-match
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
