/**
 * Entry point for every LIQUID window.
 *
 * All windows load the same bundle and branch on their Tauri window label, which
 * avoids a multi-entry build and keeps one copy of the token and glass CSS in the
 * webview cache.
 */

import React from 'react';
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

function pickRoot(label: string) {
  if (label === 'main') return <SettingsApp />;
  if (label.startsWith('overlay-') || label === 'island') return <OverlayApp label={label} />;
  // An unknown label is a bug, not a user-facing state; say so plainly rather than
  // rendering a blank window that looks like a hang.
  return (
    <div className="lq-fatal">
      <h1>Unknown window</h1>
      <p>
        No view is registered for the window label <code>{label}</code>.
      </p>
    </div>
  );
}

const container = document.getElementById('root');
if (!container) {
  throw new Error('#root is missing from index.html');
}

// Window label is synchronous in Tauri 2, so no loading state is needed.
const label = getCurrentWindow().label;
document.body.dataset['window'] = label;

createRoot(container).render(
  <React.StrictMode>{pickRoot(label)}</React.StrictMode>,
);

// Report that this window genuinely rendered. If a window looks blank or shows a
// WebView2 error page, the absence of this line in the log is the fastest way to
// tell "the page never loaded" apart from "the page loaded and threw".
ipc
  .reportFrontendReady(routeNameFor(label), window.location.href)
  .catch((e) => console.error('could not report frontend ready', e));
