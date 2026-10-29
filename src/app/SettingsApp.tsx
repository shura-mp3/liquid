/**
 * The settings window.
 *
 * Phase 1 exposes the sections that have something real behind them. Sections whose
 * subsystems do not exist yet are listed as unavailable with the phase that will
 * deliver them, rather than shown as controls that silently do nothing.
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { emit } from '@tauri-apps/api/event';

import { usePlayback } from '../hooks/usePlayback';
import {
  applyGlassToDom,
  ipc,
  type Diagnostics,
  type GlassSettings,
  type MonitorInfo,
  type OverlayStatus,
  type Settings,
} from '../providers/ipc';

type SectionId =
  | 'general'
  | 'sources'
  | 'glass'
  | 'overlays'
  | 'monitors'
  | 'hotkeys'
  | 'diagnostics'
  | 'about';

const SECTIONS: Array<{ id: SectionId; label: string; ready: boolean }> = [
  { id: 'general', label: 'General', ready: true },
  { id: 'sources', label: 'Music sources', ready: true },
  { id: 'glass', label: 'Liquid Glass', ready: true },
  { id: 'overlays', label: 'Overlays', ready: true },
  { id: 'monitors', label: 'Monitors', ready: true },
  { id: 'hotkeys', label: 'Hotkeys', ready: true },
  { id: 'diagnostics', label: 'Diagnostics', ready: true },
  { id: 'about', label: 'About & privacy', ready: true },
];

const PENDING: Array<{ label: string; phase: string }> = [
  { label: 'Lyrics', phase: 'Phase 3 — engine built and tested, not yet wired to a provider' },
  { label: 'Audio capture', phase: 'Phase 4 — WASAPI loopback' },
  { label: 'Visualisers', phase: 'Phase 4 — needs real audio first' },
  { label: 'Widgets library', phase: 'Phase 5' },
  { label: 'Themes editor', phase: 'Phase 5' },
  { label: 'Translation', phase: 'Phase 7' },
  { label: 'History & privacy controls', phase: 'Phase 7' },
];

const THEMES = [
  'liquid-midnight',
  'liquid-dark',
  'liquid-light',
  'liquid-crimson',
  'liquid-monochrome',
  'liquid-aurora',
  'album-adaptive',
];

const GLASS_CONTROLS: Array<{
  key: keyof GlassSettings;
  label: string;
  min: number;
  max: number;
  step: number;
}> = [
  { key: 'transparency', label: 'Transparency', min: 0.05, max: 1, step: 0.01 },
  { key: 'blur', label: 'Blur intensity', min: 0, max: 1, step: 0.01 },
  { key: 'frost', label: 'Frosting', min: 0, max: 1, step: 0.01 },
  { key: 'tintStrength', label: 'Album tint', min: 0, max: 1, step: 0.01 },
  { key: 'saturation', label: 'Saturation', min: 0.5, max: 2, step: 0.01 },
  { key: 'edgeGlow', label: 'Edge glow', min: 0, max: 1, step: 0.01 },
  { key: 'reflection', label: 'Reflection', min: 0, max: 1, step: 0.01 },
  { key: 'borderWidth', label: 'Border thickness', min: 0, max: 4, step: 0.1 },
  { key: 'cornerRadius', label: 'Corner radius', min: 0, max: 48, step: 1 },
  { key: 'rippleStrength', label: 'Ripple strength', min: 0, max: 1, step: 0.01 },
  { key: 'animationSpeed', label: 'Animation speed', min: 0.25, max: 3, step: 0.05 },
  { key: 'shadowSoftness', label: 'Shadow softness', min: 0, max: 1, step: 0.01 },
];

export function SettingsApp() {
  const [section, setSection] = useState<SectionId>('general');
  const [settings, setSettings] = useState<Settings | null>(null);
  const [monitors, setMonitors] = useState<MonitorInfo[]>([]);
  const [status, setStatus] = useState<OverlayStatus[]>([]);
  const [diag, setDiag] = useState<Diagnostics | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    ipc
      .getSettings()
      .then((s) => {
        setSettings(s);
        applyGlassToDom(s.glass, s.theme);
      })
      .catch((e) => setError(String(e)));
    ipc.getMonitors().then(setMonitors).catch((e) => setError(String(e)));
    ipc.getDiagnostics().then(setDiag).catch(() => {});
  }, []);

  // Overlay status is read back from the OS, so refresh it while the panel is open.
  useEffect(() => {
    if (section !== 'overlays' && section !== 'diagnostics') return;
    const tick = () => {
      ipc.overlayStatus().then(setStatus).catch(() => {});
      ipc.getDiagnostics().then(setDiag).catch(() => {});
    };
    tick();
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, [section]);

  /** Apply immediately to the DOM, persist, and tell the overlays. */
  const update = useCallback((next: Settings) => {
    setSettings(next);
    applyGlassToDom(next.glass, next.theme);
    ipc
      .saveSettings(next)
      .then(() => emit('liquid://settings-changed', next))
      .catch((e) => setError(String(e)));
  }, []);

  const setGlass = useCallback(
    (key: keyof GlassSettings, value: number) => {
      if (!settings) return;
      update({ ...settings, glass: { ...settings.glass, [key]: value } });
    },
    [settings, update],
  );

  const pending = useMemo(() => PENDING, []);

  if (!settings) {
    return (
      <div className="lq-shell lq-shell--loading">
        <div className="lq-spinner" aria-label="Loading" />
        {error && <p className="lq-error">{error}</p>}
      </div>
    );
  }

  return (
    <div className="lq-shell">
      <aside className="lq-nav">
        <div className="lq-brand">
          <span className="lq-brand__mark" aria-hidden="true" />
          <div>
            <strong>LIQUID</strong>
            <small>v{diag?.appVersion ?? '0.1.0'}</small>
          </div>
        </div>
        <nav>
          {SECTIONS.map((s) => (
            <button
              key={s.id}
              type="button"
              className="lq-nav__item"
              data-active={section === s.id}
              onClick={() => setSection(s.id)}
            >
              {s.label}
            </button>
          ))}
        </nav>
        <div className="lq-nav__pending">
          <h4>Not built yet</h4>
          {pending.map((p) => (
            <div key={p.label} className="lq-nav__pending-row">
              <span>{p.label}</span>
              <small>{p.phase}</small>
            </div>
          ))}
        </div>
      </aside>

      <main className="lq-main">
        {error && (
          <div className="lq-banner lq-banner--error">
            <strong>Something failed:</strong> {error}
            <button type="button" className="lq-btn lq-btn--ghost" onClick={() => setError(null)}>
              Dismiss
            </button>
          </div>
        )}

        {section === 'general' && (
          <Panel title="General" subtitle="Appearance, startup and performance.">
            <Field label="Theme" hint="Semantic tokens, so every widget follows along.">
              <select
                className="lq-input"
                value={settings.theme}
                onChange={(e) => update({ ...settings, theme: e.target.value })}
              >
                {THEMES.map((t) => (
                  <option key={t} value={t}>
                    {t.replace(/-/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase())}
                  </option>
                ))}
              </select>
            </Field>

            <Field
              label="Frame rate cap"
              hint="Lower caps reduce GPU load. 30 is the battery-saver choice."
            >
              <select
                className="lq-input"
                value={settings.fpsCap}
                onChange={(e) => update({ ...settings, fpsCap: Number(e.target.value) })}
              >
                {[30, 60, 120, 144, 165, 240].map((f) => (
                  <option key={f} value={f}>
                    {f} FPS
                  </option>
                ))}
              </select>
            </Field>

            <Field label="Performance preset">
              <select
                className="lq-input"
                value={settings.performancePreset}
                onChange={(e) => update({ ...settings, performancePreset: e.target.value })}
              >
                {['ultra', 'high', 'balanced', 'gaming', 'battery-saver'].map((p) => (
                  <option key={p} value={p}>
                    {p.replace(/-/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase())}
                  </option>
                ))}
              </select>
            </Field>

            <Toggle
              label="Launch LIQUID when I sign in"
              hint="Registered per-user under HKCU\Run. No administrator rights, no scheduled task."
              checked={settings.launchAtLogin}
              onChange={(v) => {
                ipc
                  .setLaunchAtLogin(v)
                  .then(() => setSettings({ ...settings, launchAtLogin: v }))
                  .catch((e) => setError(String(e)));
              }}
            />

            <Toggle
              label="Start minimised to the tray"
              checked={settings.startMinimised}
              onChange={(v) => update({ ...settings, startMinimised: v })}
            />

            <div className="lq-row-end">
              <button
                type="button"
                className="lq-btn lq-btn--ghost"
                onClick={() => {
                  ipc
                    .resetSettings()
                    .then((s) => {
                      setSettings(s);
                      applyGlassToDom(s.glass, s.theme);
                      return emit('liquid://settings-changed', s);
                    })
                    .catch((e) => setError(String(e)));
                }}
              >
                Reset everything to defaults
              </button>
            </div>
          </Panel>
        )}

        {section === 'sources' && <SourcesPanel />}

        {section === 'glass' && (
          <Panel
            title="Liquid Glass"
            subtitle="Changes apply live. Every value is written straight into CSS custom properties."
          >
            <div className="lq-glass lq-preview" data-tinted="true">
              <strong>Preview</strong>
              <p>The quick brown fox jumps over the lazy dog.</p>
              <small>Readability at this contrast is what matters, not maximum blur.</small>
            </div>

            <div className="lq-sliders">
              {GLASS_CONTROLS.map((c) => (
                <label key={c.key} className="lq-slider">
                  <span className="lq-slider__label">
                    {c.label}
                    <em>{Number(settings.glass[c.key]).toFixed(2)}</em>
                  </span>
                  <input
                    type="range"
                    min={c.min}
                    max={c.max}
                    step={c.step}
                    value={settings.glass[c.key]}
                    onChange={(e) => setGlass(c.key, Number(e.target.value))}
                  />
                </label>
              ))}
            </div>

            <div className="lq-note">
              <strong>What this effect really is.</strong> A transparent overlay cannot read
              the pixels of windows behind it — no Windows API hands it that backdrop, and
              capturing the screen continuously just to fake refraction would waste GPU
              bandwidth and trip protected-content rules. So this is a layered approximation:
              real <code>backdrop-filter</code> blur where the compositor allows it, plus a
              procedural frost grain, a specular sweep and a rim light. It is not sampling
              your desktop.
            </div>
          </Panel>
        )}

        {section === 'overlays' && (
          <Panel title="Overlays" subtitle="Live state, read back from the operating system.">
            <Toggle
              label="Show the overlay"
              checked={settings.overlayVisible}
              onChange={(v) => {
                ipc
                  .setOverlayVisible(v)
                  .then(() => setSettings({ ...settings, overlayVisible: v }))
                  .catch((e) => setError(String(e)));
              }}
            />
            <Toggle
              label="Click-through (the overlay ignores the mouse)"
              hint={`Always recoverable: ${settings.hotkeys['toggleClickThrough'] ?? 'Ctrl+Alt+C'} or the tray menu restores it.`}
              checked={settings.clickThrough}
              onChange={(v) => {
                ipc
                  .setClickThrough(v)
                  .then((applied) => setSettings({ ...settings, clickThrough: applied }))
                  .catch((e) => setError(String(e)));
              }}
            />

            <div className="lq-row-end">
              <button
                type="button"
                className="lq-btn"
                onClick={() => ipc.restoreInteractivity().catch((e) => setError(String(e)))}
              >
                Restore interactivity now
              </button>
            </div>

            <h3 className="lq-subhead">Overlay windows</h3>
            {status.length === 0 ? (
              <p className="lq-muted">No overlay windows reported.</p>
            ) : (
              <table className="lq-table">
                <thead>
                  <tr>
                    <th>Window</th>
                    <th>Topmost</th>
                    <th>Styles</th>
                    <th>Click-through</th>
                    <th>Visible</th>
                    <th>Obstruction</th>
                  </tr>
                </thead>
                <tbody>
                  {status.map((s) => (
                    <tr key={s.label}>
                      <td>
                        <code>{s.label}</code>
                      </td>
                      <td>{s.topmost ? 'yes' : 'no'}</td>
                      <td>
                        {s.stylesOk ? (
                          <span className="lq-ok">ok</span>
                        ) : (
                          <span className="lq-warn">incomplete</span>
                        )}
                      </td>
                      <td>{s.clickThrough ? 'yes' : 'no'}</td>
                      <td>{s.visible ? 'yes' : 'no'}</td>
                      <td>
                        {s.obstruction === 'none' ? (
                          <span className="lq-muted">none</span>
                        ) : (
                          <span className="lq-warn">suspected exclusive fullscreen</span>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}

            <div className="lq-note">
              <strong>Where an overlay cannot appear.</strong> Exclusive-fullscreen games,
              UAC prompts and the lock screen, DRM-protected video, and some elevated or
              anti-cheat-protected windows will cover LIQUID. That is by design in Windows,
              and LIQUID does not inject into other processes or try to defeat it. For games,
              switching the game to borderless windowed is the fix.
            </div>
          </Panel>
        )}

        {section === 'monitors' && (
          <Panel
            title="Monitors"
            subtitle="Enumerated with per-monitor DPI. One overlay window is created per display."
          >
            <table className="lq-table">
              <thead>
                <tr>
                  <th>Device</th>
                  <th>Position</th>
                  <th>Size</th>
                  <th>Work area</th>
                  <th>Scale</th>
                  <th>Primary</th>
                </tr>
              </thead>
              <tbody>
                {monitors.map((m) => (
                  <tr key={m.id}>
                    <td>
                      <code>{m.id}</code>
                    </td>
                    <td>
                      {m.x}, {m.y}
                    </td>
                    <td>
                      {m.width} &times; {m.height}
                    </td>
                    <td>
                      {m.work_width} &times; {m.work_height}
                    </td>
                    <td>{m.scale_factor.toFixed(2)}&times;</td>
                    <td>{m.is_primary ? 'yes' : 'no'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <p className="lq-muted">
              Coordinates can legitimately be negative when a display sits left of or above
              the primary one. Widgets left on a disconnected monitor are moved back into
              reach automatically.
            </p>
          </Panel>
        )}

        {section === 'hotkeys' && (
          <Panel
            title="Hotkeys"
            subtitle="Registered globally at startup. A combination already owned by another app is reported in the log rather than silently stolen."
          >
            <table className="lq-table">
              <thead>
                <tr>
                  <th>Action</th>
                  <th>Combination</th>
                </tr>
              </thead>
              <tbody>
                {Object.entries(settings.hotkeys).map(([action, combo]) => (
                  <tr key={action}>
                    <td>{action.replace(/([A-Z])/g, ' $1').toLowerCase()}</td>
                    <td>
                      <input
                        className="lq-input lq-input--compact"
                        value={combo}
                        onChange={(e) =>
                          update({
                            ...settings,
                            hotkeys: { ...settings.hotkeys, [action]: e.target.value },
                          })
                        }
                      />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            <p className="lq-muted">
              Rebinding takes effect after a restart in this build; live re-registration
              lands with the hotkey editor.
            </p>
          </Panel>
        )}

        {section === 'diagnostics' && diag && (
          <Panel title="Diagnostics" subtitle="Measured values. Nothing here is estimated.">
            <dl className="lq-kv lq-kv--wide">
              <dt>Version</dt>
              <dd>{diag.appVersion}</dd>
              <dt>Monotonic clock</dt>
              <dd>
                {diag.monotonicClockDegraded ? (
                  <span className="lq-warn">
                    degraded — QueryPerformanceCounter unavailable, timing will be less accurate
                  </span>
                ) : (
                  <span className="lq-ok">QueryPerformanceCounter, sub-millisecond</span>
                )}
              </dd>
              <dt>Clock reading</dt>
              <dd>{(diag.monotonicNowMs / 1000).toFixed(3)} s since start</dd>
              <dt>Monitors</dt>
              <dd>{diag.monitorCount}</dd>
              <dt>Launch at login</dt>
              <dd>{diag.launchesAtLogin ? 'registered' : 'not registered'}</dd>
              <dt>Media session</dt>
              <dd className="lq-muted">{diag.mediaSession}</dd>
              <dt>Audio capture</dt>
              <dd className="lq-muted">{diag.audioCapture}</dd>
              <dt>Artwork</dt>
              <dd>
                {diag.artworkCached ? 'cached for the current track' : 'none for the current track'}
                {` · ${diag.artworkCacheEntries} in cache (bounded)`}
              </dd>
              <dt>Lyrics cache</dt>
              <dd>{diag.lyricsCacheEntries} track{diag.lyricsCacheEntries === 1 ? '' : 's'}</dd>
            </dl>
            <p className="lq-muted">
              Logs are written to the LIQUID folder in your app-data directory. They stay on
              this machine; nothing is uploaded.
            </p>
          </Panel>
        )}

        {section === 'about' && (
          <Panel title="About & privacy" subtitle="What LIQUID does and does not do.">
            <div className="lq-note">
              <strong>No telemetry.</strong> LIQUID collects nothing and sends nothing about
              you anywhere. Listening history, when enabled, is stored only in the local
              SQLite database and can be disabled or deleted.
            </div>
            <div className="lq-note">
              <strong>Streaming services.</strong> LIQUID integrates no Spotify API, SDK or
              endpoint — there is no Spotify sign-in, client ID or key anywhere in it. Track
              information comes from Windows media sessions, the same operating-system
              facility that drives the volume-key popup, and it works the same way for any
              media app.
            </div>
            <div className="lq-note">
              <strong>Audio analysis.</strong> Visualisers read the Windows system audio mix.
              On Windows 10 that mix cannot be separated per application, so anything else
              making noise — a game, a call, a notification — is part of what you see.
            </div>
            <div className="lq-note">
              <strong>Lyrics.</strong> Sourced from your own files or from LRCLIB, matched on
              title, artist, album and duration. A candidate whose length differs by more
              than two seconds, or which is marked as a different version such as a live or
              sped-up take, is refused rather than applied to the wrong recording.
            </div>
            <p className="lq-muted">
              Source and full compatibility notes:{' '}
              <code>github.com/shura-mp3/liquid</code>
            </p>
          </Panel>
        )}
      </main>
    </div>
  );
}

/**
 * Music sources. Everything here is read back from the Windows media session
 * service, including the capability flags, so a disabled control reflects what the
 * source actually said rather than what we hope it supports.
 */
function SourcesPanel() {
  const playback = usePlayback();
  const [busy, setBusy] = useState<string | null>(null);

  const pin = (sourceApp: string | null) => {
    setBusy(sourceApp ?? 'clear');
    ipc
      .setPinnedSource(sourceApp)
      .catch((e) => console.error(e))
      .finally(() => setBusy(null));
  };

  const caps = playback.snapshot?.controls;
  const bridgeDown = playback.status?.bridgeRunning === false;

  return (
    <Panel
      title="Music sources"
      subtitle="Read from Windows media sessions — the same facility that drives the volume-key popup. No streaming-service API is involved."
    >
      {bridgeDown && (
        <div className="lq-banner lq-banner--error">{playback.status?.error}</div>
      )}

      <div className="lq-field">
        <div className="lq-field__text">
          <span>Active source</span>
          <small>
            {playback.snapshot
              ? `${playback.snapshot.title}${playback.snapshot.artist ? ` — ${playback.snapshot.artist}` : ''}`
              : 'Nothing is playing right now.'}
          </small>
        </div>
        <span className="lq-pill" data-state={playback.snapshot?.status ?? 'none'}>
          {playback.snapshot?.status ?? 'idle'}
        </span>
      </div>

      <div className="lq-row-end">
        <button
          type="button"
          className="lq-btn lq-btn--ghost"
          onClick={() => ipc.refreshMediaSessions().catch((e) => console.error(e))}
        >
          Refresh sessions
        </button>
      </div>

      <h3 className="lq-subhead">Detected sessions</h3>
      {playback.sessions.length === 0 ? (
        <p className="lq-muted">
          No application is currently publishing a media session. Start playback in
          Spotify, a browser or any local player and it will appear here.
        </p>
      ) : (
        <table className="lq-table">
          <thead>
            <tr>
              <th>Application</th>
              <th>Track</th>
              <th>State</th>
              <th>Follow</th>
            </tr>
          </thead>
          <tbody>
            {playback.sessions.map((s) => (
              <tr key={s.sourceApp}>
                <td>
                  {s.displayName}
                  {s.isCurrent && <span className="lq-tag">system current</span>}
                </td>
                <td>{s.title ?? <span className="lq-muted">no metadata</span>}</td>
                <td>{s.status}</td>
                <td>
                  <button
                    type="button"
                    className="lq-btn lq-btn--ghost"
                    disabled={busy !== null}
                    onClick={() => pin(s.isPinned ? null : s.sourceApp)}
                  >
                    {s.isPinned ? 'Unpin' : 'Pin'}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <h3 className="lq-subhead">Reported capabilities</h3>
      {caps ? (
        <div className="lq-caps">
          {(
            [
              ['play', caps.play],
              ['pause', caps.pause],
              ['next', caps.next],
              ['previous', caps.previous],
              ['seek', caps.seek],
              ['volume', caps.volume],
            ] as const
          ).map(([name, ok]) => (
            <span key={name} className="lq-cap" data-ok={ok}>
              {name}
            </span>
          ))}
        </div>
      ) : (
        <p className="lq-muted">No active source to report capabilities for.</p>
      )}
      <p className="lq-muted">
        These come from the source, not from LIQUID. Browsers commonly expose
        next/previous but not seek, and Windows media sessions expose no volume
        control at all — so that one is always off. Controls LIQUID cannot use are
        disabled with the reason in their tooltip rather than silently doing nothing.
      </p>

      <div className="lq-note">
        <strong>About Spotify.</strong> LIQUID integrates no Spotify API, SDK or
        endpoint, and there is no sign-in. Spotify appears here only because the
        desktop app publishes a Windows media session like any other player.
      </div>
    </Panel>
  );
}

function Panel({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="lq-panel">
      <header className="lq-panel__head">
        <h2>{title}</h2>
        {subtitle && <p>{subtitle}</p>}
      </header>
      <div className="lq-panel__body">{children}</div>
    </section>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="lq-field">
      <div className="lq-field__text">
        <span>{label}</span>
        {hint && <small>{hint}</small>}
      </div>
      <div className="lq-field__control">{children}</div>
    </div>
  );
}

function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <label className="lq-field lq-field--toggle">
      <div className="lq-field__text">
        <span>{label}</span>
        {hint && <small>{hint}</small>}
      </div>
      <span className="lq-switch" data-on={checked}>
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => onChange(e.target.checked)}
        />
        <span className="lq-switch__thumb" />
      </span>
    </label>
  );
}
