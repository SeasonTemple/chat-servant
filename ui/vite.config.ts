import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

const host = process.env.TAURI_DEV_HOST;

// Tauri 开发时需要一个固定的 dev server 地址，见 https://v2.tauri.app/start/frontend/basics
export default defineConfig(async () => ({
  plugins: [sveltekit()],

  // 防止 Tauri 在 Windows 上把源码目录整个暴露出去
  clearScreen: false,

  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: {
      // Rust 侧不归 Vite 管
      ignored: ['**/src-tauri/**'],
    },
  },
}));
