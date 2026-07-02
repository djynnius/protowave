import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  server: {
    fs: {
      // The canonical .proto schema is imported (?raw) from crates/.
      allow: ['..'],
    },
    proxy: {
      '/ws': {
        target: 'ws://127.0.0.1:9898',
        ws: true,
      },
    },
  },
})
