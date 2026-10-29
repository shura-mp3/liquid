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
import { shouldTrustWordSync } from '../core/clock';

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
  const lyrics = useDraggableWidget({ x: 48, y: 420 }, locked);

  if (!settings) return null;

  const sourceName =
    playback.sessions.find((s) => s.sourceApp === playback.snapshot?.sourceApp)
      ?.displayName ??
    playback.snapshot?.sourceApp ??
    null;

  const trusted = playback.reading ? shouldTrustWordSync(playback.reading) : false;

  return (
    <div className="lq-overlay-root" data-chrome={showChrome}>
      {/* Now playing — real data from the Windows media session. */}
      <div
        ref={nowPlaying.ref}
        className="lq-glass lq-widget lq-widget--np"
        data-interactive={!clickThrough}
        data-tinted="true"
        style={{ transform: `translate3d(${nowPlaying.pos.x}px, ${nowPlaying.pos.y}px, 0)` }}
        {...nowPlaying.handlers}
      >
        <header className="lq-widget__bar">
          <span className="lq-widget__title">Now playing</span>
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

      {/* Lyrics — engine ready, provider not yet wired. Labelled, not faked. */}
      <div
        ref={lyrics.ref}
        className="lq-glass lq-widget lq-widget--lyrics"
        data-interactive={!clickThrough}
        data-tinted="true"
        style={{ transform: `translate3d(${lyrics.pos.x}px, ${lyrics.pos.y}px, 0)` }}
        {...lyrics.handlers}
      >
        <header className="lq-widget__bar">
          <span className="lq-widget__title">Lyrics</span>
          <span className="lq-pill lq-pill--pending">awaiting Phase 3</span>
        </header>

        {playback.snapshot ? (
          <div className="lq-lyrics">
            <p className="lq-lyrics__line lq-lyrics__line--active">
              {playback.snapshot.title}
            </p>
            <p className="lq-lyrics__line lq-lyrics__line--future">
              No lyrics provider is connected yet.
            </p>
          </div>
        ) : (
          <p className="lq-muted">Waiting for a track.</p>
        )}

        {/* The timing engine is already running against the live clock, so its
            real numbers are shown rather than a mock-up of them. */}
        <dl className="lq-kv lq-kv--timing">
          <dt>Position</dt>
          <dd>{Math.round(playback.reading?.positionMs ?? 0)} ms</dd>
          <dt>Uncertainty</dt>
          <dd>
            <span className={trusted ? 'lq-ok' : 'lq-warn'}>
              &plusmn;{Math.round(playback.reading?.uncertaintyMs ?? 0)} ms
            </span>
          </dd>
          <dt>Source timing</dt>
          <dd>{playback.snapshot?.timingConfidence ?? 'unknown'}</dd>
          <dt>Last event</dt>
          <dd>{playback.lastDiscontinuity ?? 'none'}</dd>
          <dt>Window</dt>
          <dd>{label}</dd>
        </dl>
      </div>
    </div>
  );
}
