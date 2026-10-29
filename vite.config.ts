import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath, URL } from 'node:url';

// Tauri expects a fixed port and must fail rather than silently pick another one,
// otherwise the webview loads nothing.
const host = process.env.TAURI_DEV_HOST;

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
    host: host || false,
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
  test: {
    environment: 'node',
    include: ['tests/**/*.test.ts'],
  },
});
