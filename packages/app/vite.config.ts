import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { resolve } from 'node:path';
import { bridgeProxyConfig } from './vite-bridge-proxy';

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': resolve(__dirname, 'src'),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: true,
    allowedHosts: true,
    proxy: bridgeProxyConfig,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  preview: {
    host: true,
    allowedHosts: true,
    proxy: bridgeProxyConfig,
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: 'es2022',
    minify: !process.env.TAURI_DEBUG ? 'esbuild' : false,
    sourcemap: !!process.env.TAURI_DEBUG,
    rollupOptions: {
      output: {
        // Split the big vendor libraries out of the app bundle so each chunk stays small.
        manualChunks(id: string) {
          if (!id.includes('node_modules')) return undefined;
          if (id.includes('/@xterm/')) return 'vendor-xterm';
          if (id.includes('/qrcode/') || id.includes('/html5-qrcode/')) return 'vendor-qr';
          if (id.includes('/@tauri-apps/')) return 'vendor-tauri';
          if (/\/node_modules\/(react|react-dom|scheduler)\//.test(id)) return 'vendor-react';
          return undefined;
        },
      },
    },
  },
});
