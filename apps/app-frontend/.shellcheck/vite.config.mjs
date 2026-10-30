import vue from '@vitejs/plugin-vue'
import { resolve } from 'path'
import { defineConfig } from 'vite'

// 临时校验配置：只把 src/ui 外壳层单独编译一遍，确认 SFC / CSS 真的能过。
export default defineConfig({
    plugins: [vue()],
    build: {
        outDir: '.shellcheck/dist',
        emptyOutDir: true,
        lib: {
            entry: resolve(__dirname, 'entry.ts'),
            formats: ['es'],
            fileName: 'shell',
        },
        rollupOptions: {
            external: ['vue', '@tauri-apps/api/window', '@tauri-apps/api/core'],
        },
    },
})
