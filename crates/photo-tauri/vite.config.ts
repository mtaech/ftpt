import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

// Tauri 开发约定：固定 1420 端口（src-tauri 的 devUrl 指向它），strictPort 防止漂移
export default defineConfig({
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  // 防止终端清屏遮挡 tauri 侧输出
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: 'dist',
  },
  envPrefix: ['VITE_', 'TAURI_'],
  test: {
    server: {
      deps: {
        // @material/material-color-utilities 深部 import 无 .js 扩展名，
        // Node ESM 严格解析会失败（vitest SSR 外部化）；走 vite 转换管线即可
        inline: [/@material\/material-color-utilities/],
      },
    },
  },
})
