import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
export default {
  preprocess: vitePreprocess(),
  kit: {
    // Single-page app: the Rust server serves index.html for client routes.
    adapter: adapter({ fallback: 'index.html', strict: false })
  }
};
