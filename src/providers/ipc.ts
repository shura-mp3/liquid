/**
 * Typed IPC client for the Rust commands.
 *
 * One module owns every `invoke` call so the command names and payload shapes exist
 * in exactly one place. These types mirror the serde definitions in
 * `src-tauri/src/commands.rs` and `src-tauri/src/storage/settings.rs`.
 */

import { invoke } from '@tauri-apps/api/core';

export interface MonitorInfo {
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
  work_x: number;
  work_y: number;
  work_width: number;
  work_height: number;
  is_primary: boolean;
  scale_factor: number;
}

export interface GlassSettings {
  transparency: number;
  blur: number;
  frost: number;
  tintStrength: number;
  saturation: number;
  edgeGlow: number;
  reflection: number;
  borderWidth: number;
  cornerRadius: number;
  rippleStrength: number;
  animationSpeed: number;
  shadowSoftness: number;
}

export interface Settings {
  theme: string;
  performancePreset: string;
  fpsCap: number;
  clickThrough: boolean;
  overlayVisible: boolean;
  launchAtLogin: boolean;
  startMinimised: boolean;
  globalOffsetMs: number;
  spotifyAudioReactiveVisuals: boolean;
  historyEnabled: boolean;
  glass: GlassSettings;
  hotkeys: Record<string, string>;
}

export type OverlayObstruction = 'none' | 'suspected-exclusive-fullscreen';

export interface OverlayStatus {
  label: string;
  topmost: boolean;
  clickThrough: boolean;
  visible: boolean;
  /**
   * Whether the required extended-style set is currently present. tao rewrites the
   * whole style word when it re-applies its own window flags, so this is surfaced
   * rather than assumed.
   */
  stylesOk: boolean;
  obstruction: OverlayObstruction;
}

export interface Diagnostics {
  appVersion: string;
  monotonicClockDegraded: boolean;
  monotonicNowMs: number;
  monitorCount: number;
  launchesAtLogin: boolean;
  audioCapture: string;
  mediaSession: string;
}

export const ipc = {
  getMonitors: () => invoke<MonitorInfo[]>('get_monitors'),
  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings) => invoke<void>('save_settings', { settings }),
  resetSettings: () => invoke<Settings>('reset_settings'),
  setClickThrough: (enabled: boolean) => invoke<boolean>('set_click_through', { enabled }),
  toggleClickThrough: () => invoke<boolean>('toggle_click_through'),
  restoreInteractivity: () => invoke<void>('restore_interactivity'),
  setOverlayVisible: (visible: boolean) => invoke<void>('set_overlay_visible', { visible }),
  assertTopmost: () => invoke<void>('assert_topmost'),
  overlayStatus: () => invoke<OverlayStatus[]>('overlay_status'),
  getDiagnostics: () => invoke<Diagnostics>('get_diagnostics'),
  setLaunchAtLogin: (enabled: boolean) => invoke<void>('set_launch_at_login', { enabled }),
  showSettingsWindow: () => invoke<void>('show_settings_window'),
  /**
   * Tell the native side the page actually loaded. Window creation succeeding does
   * not mean the webview rendered anything, so this is what proves it.
   */
  reportFrontendReady: (route: string, href: string) =>
    invoke<void>('report_frontend_ready', { route, href }),
};

/**
 * Push glass settings into CSS custom properties so slider changes apply instantly
 * without a reload. The CSS consumes these; no component reads them directly.
 */
export function applyGlassToDom(glass: GlassSettings, theme: string): void {
  const root = document.documentElement;
  root.dataset['theme'] = theme;
  root.style.setProperty('--lq-glass-alpha', String(glass.transparency));
  root.style.setProperty('--lq-glass-blur', `${(glass.blur * 34).toFixed(1)}px`);
  root.style.setProperty('--lq-glass-frost', String(glass.frost));
  root.style.setProperty('--lq-glass-saturation', String(glass.saturation));
  root.style.setProperty('--lq-glass-tint-strength', String(glass.tintStrength));
  root.style.setProperty('--lq-glass-edge-glow', String(glass.edgeGlow));
  root.style.setProperty('--lq-glass-reflection', String(glass.reflection));
  root.style.setProperty('--lq-glass-border-width', `${glass.borderWidth.toFixed(2)}px`);
  root.style.setProperty('--lq-glass-radius', `${glass.cornerRadius.toFixed(0)}px`);
  root.style.setProperty('--lq-glass-shadow-softness', String(glass.shadowSoftness));
  // Guard against zero: it would make every duration division infinite.
  root.style.setProperty('--lq-anim-speed', String(Math.max(0.1, glass.animationSpeed)));
}
