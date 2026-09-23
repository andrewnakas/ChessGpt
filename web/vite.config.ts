import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    port: 5173,
    proxy: {
      // The Rust API; SSE streams pass through unbuffered.
      '/api': { target: 'http://127.0.0.1:8080', changeOrigin: false }
    }
  },
  test: { include: ['src/**/*.test.ts'] }
});
