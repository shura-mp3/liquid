import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath, URL } from 'node:url';

// Tauri expects a fixed port and must fail rather than silently pick another one,
// otherwise the webview loads nothing.
const host = process.env.TAURI_DEV_HOST;

/**
 * Bind the dev server to the IPv4 loopback explicitly.
 *
 * Vite's default host is the *name* `localhost`, which Node 17+ resolves via the OS
 * resolver. On this machine `localhost` resolves to `::1` first, so Vite bound only
 * to `[::1]:5183` and nothing was listening on `127.0.0.1:5183`. WebView2 resolves
 * `localhost` to the IPv4 address, so every window loaded `ERR_CONNECTION_REFUSED`
 * while `curl http://localhost:5183` succeeded — the two clients disagreed about
 * what `localhost` means.
 *
 * Pinning both sides to a literal `127.0.0.1` removes the name-resolution step
 * entirely. Binding to `true` / `0.0.0.0` would also have worked, but that exposes
 * the dev server to the whole network, which is not an acceptable way to fix a
 * loopback bug. `TAURI_DEV_HOST` still overrides for genuine LAN/device testing.
 */
const devHost = host || '127.0.0.1';

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  clearScreen: false,
  server: {
    port: 5183,
    strictPort: true,
    host: devHost,
    hmr: host ? { protocol: 'ws', host, port: 5184 } : undefined,
    watch: {
      // src-tauri churns during cargo builds; watching it causes reload storms.
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    // WebView2 on Win10 ships a modern Chromium; no legacy transpilation needed.
    target: 'chrome110',
    minify: !process.env.TAURI_ENV_DEBUG ? 'esbuild' : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
