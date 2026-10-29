/**
 * The overlay window.
 *
 * Phase 1 scope: prove the window itself works — transparent, topmost, draggable
 * widgets, click-through with a visible state, and correct behaviour at each
 * monitor's own DPI. The widgets here show real window state and placeholder *track*
 * state that is explicitly labelled as such, because no media provider exists until
 * Phase 2 and inventing a fake "now playing" would make the overlay look finished
 * when it is not.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';

import { applyGlassToDom, ipc, type Settings } from '../providers/ipc';

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
      // Do not start a drag from a control inside the widget.
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

  // Hotkey-driven actions arrive as events from the native layer.
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

  /**
   * Re-assert topmost when this window loses focus.
   *
   * Other applications take the topmost slot legitimately and silently push the
   * overlay behind them. This is event-driven rather than a polling timer, which
   * would burn CPU forever for a condition that changes rarely.
   */
  useEffect(() => {
    const win = getCurrentWindow();
    const promise = win.onFocusChanged(({ payload: focused }) => {
      if (!focused) ipc.assertTopmost().catch(() => {});
    });
    return () => {
      promise.then((un) => un()).catch(() => {});
    };
  }, []);

  const lyrics = useDraggableWidget({ x: 48, y: 120 }, locked);
  const status = useDraggableWidget({ x: 48, y: 380 }, locked);

  if (!settings) {
    return null;
  }

  return (
    <div className="lq-overlay-root" data-chrome={showChrome}>
      {/* Lyrics widget */}
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
        <div className="lq-lyrics">
          <p className="lq-lyrics__line lq-lyrics__line--past">
            the timing engine is built and tested
          </p>
          <p className="lq-lyrics__line lq-lyrics__line--active">
            no media provider is connected yet
          </p>
          <p className="lq-lyrics__line lq-lyrics__line--future">
            so there is nothing real to display
          </p>
        </div>
        <footer className="lq-widget__note">
          Placeholder text, not a track. Real lyrics arrive with the provider in Phase 2.
        </footer>
      </div>

      {/* Overlay status widget — this part shows genuine live state. */}
      <div
        ref={status.ref}
        className="lq-glass lq-widget lq-widget--status"
        data-interactive={!clickThrough}
        style={{ transform: `translate3d(${status.pos.x}px, ${status.pos.y}px, 0)` }}
        {...status.handlers}
      >
        <header className="lq-widget__bar">
          <span className="lq-widget__title">Overlay</span>
          <span className="lq-pill lq-pill--live">live</span>
        </header>
        <dl className="lq-kv">
          <dt>Window</dt>
          <dd>
            <code>{label}</code>
          </dd>
          <dt>Scale</dt>
          <dd>{window.devicePixelRatio.toFixed(2)}&times;</dd>
          <dt>Viewport</dt>
          <dd>
            {window.innerWidth} &times; {window.innerHeight}
          </dd>
          <dt>Click-through</dt>
          <dd>{clickThrough ? 'on' : 'off'}</dd>
        </dl>
        <div className="lq-widget__actions lq-no-drag">
          <button
            type="button"
            className="lq-btn"
            onClick={() => {
              ipc
                .toggleClickThrough()
                .then(setClickThrough)
                .catch((e) => console.error(e));
            }}
          >
            {clickThrough ? 'Disable' : 'Enable'} click-through
          </button>
          <button type="button" className="lq-btn" onClick={() => setLocked((v) => !v)}>
            {locked ? 'Unlock' : 'Lock'} widgets
          </button>
          <button
            type="button"
            className="lq-btn lq-btn--ghost"
            onClick={() => ipc.showSettingsWindow().catch((e) => console.error(e))}
          >
            Settings
          </button>
        </div>
        {clickThrough && (
          <p className="lq-widget__note lq-widget__note--warn">
            Click-through is on, so this panel ignores the mouse. Press
            {' '}
            <kbd>{settings.hotkeys['toggleClickThrough'] ?? 'Ctrl+Alt+C'}</kbd> or use the
            tray icon to get it back.
          </p>
        )}
      </div>
    </div>
  );
}
