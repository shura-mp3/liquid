/**
 * Entry point for every LIQUID window.
 *
 * All windows load the same bundle and branch on their Tauri window label.
 *
 * Readiness is reported **only after a successful mount**. That signal is what
 * allows the native side to show an overlay, so a window whose frontend throws
 * stays hidden rather than appearing as a blank sheet over the desktop. The
 * watchdog on the Rust side then fails it and reports why.
 */

import React, { useEffect } from 'react';
import { createRoot } from 'react-dom/client';
import { getCurrentWindow } from '@tauri-apps/api/window';

import '../styles/tokens.css';
import '../styles/glass.css';
import '../styles/app.css';

import { SettingsApp } from './SettingsApp';
import { OverlayApp } from './OverlayApp';
import { ipc } from '../providers/ipc';

/** Which view a window label resolves to. Logged so the route is verifiable. */
function routeNameFor(label: string): string {
  if (label === 'main') return 'settings';
  if (label.startsWith('overlay-')) return 'overlay';
  if (label === 'island') return 'island';
  return 'unknown';
}

const label = getCurrentWindow().label;
const isOverlay = label.startsWith('overlay-') || label === 'island';

/**
 * Stops one component's exception from blanking the window.
 *
 * In the settings window a visible error is far more useful than a white page. In
 * an overlay there is nowhere sensible to put an error, and a panel of red text
 * floating over the desktop is worse than nothing — so it renders nothing and
 * leaves the window transparent, while still reporting to the log.
 */
class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error('LIQUID render error', error, info);
  }

  render() {
    if (!this.state.error) return this.props.children;
    if (isOverlay) return null;
    return (
      <div className="lq-fatal">
        <h1>Something broke</h1>
        <p>{this.state.error.message}</p>
        <p className="lq-muted">
          The rest of LIQUID is still running. Details are in the log.
        </p>
      </div>
    );
  }
}

/**
 * Signals readiness once React has committed.
 *
 * Deliberately a mount effect rather than `requestAnimationFrame`. An overlay is
 * created off-screen so it can load without being able to cover anything, and a
 * browser suspends rAF for a window that is not being composited — so an
 * rAF-based signal never fires, the window is never shown, and the watchdog
 * eventually declares a perfectly healthy overlay failed. `useEffect` runs on
 * commit, visible or not.
 *
 * A timeout backstop covers the case where an effect somehow does not run; it is
 * cancelled as soon as the normal path reports.
 */
function ReadySignal({ label: windowLabel }: { label: string }) {
  useEffect(() => {
    let reported = false;
    const report = () => {
      if (reported) return;
      reported = true;
      ipc
        .reportFrontendReady(routeNameFor(windowLabel), window.location.href)
        .catch((e) => console.error('could not report frontend ready', e));
    };
    report();
    const backstop = window.setTimeout(report, 2000);
    return () => window.clearTimeout(backstop);
  }, [windowLabel]);
  return null;
}

function pickRoot(windowLabel: string) {
  if (windowLabel === 'main') return <SettingsApp />;
  if (isOverlay) return <OverlayApp label={windowLabel} />;
  return (
    <div className="lq-fatal">
      <h1>Unknown window</h1>
      <p>
        No view is registered for the window label <code>{windowLabel}</code>.
      </p>
    </div>
  );
}

const container = document.getElementById('root');
if (!container) {
  throw new Error('#root is missing from index.html');
}

document.body.dataset['window'] = label;

// Surface anything that escapes React, so a silent failure is never the outcome.
window.addEventListener('error', (e) => console.error('LIQUID uncaught error', e.error ?? e.message));
window.addEventListener('unhandledrejection', (e) =>
  console.error('LIQUID unhandled rejection', e.reason),
);

createRoot(container).render(
  <React.StrictMode>
    {/* Outside the boundary: a crashing view must still report in, so the native
        side learns the page loaded rather than waiting out the whole timeout. */}
    <ReadySignal label={label} />
    <ErrorBoundary>{pickRoot(label)}</ErrorBoundary>
  </React.StrictMode>,
);
