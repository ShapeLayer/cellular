import { defineConfig } from 'vite';

/**
 * Relative asset paths keep one build working from a domain root (Cloudflare
 * Pages) and from a repository subpath (GitHub Pages) alike. Override with
 * VITE_BASE when a deployment needs an absolute prefix.
 */
export default defineConfig({
  base: process.env.VITE_BASE ?? './',
  build: {
    outDir: 'dist',
    target: 'es2022',
    sourcemap: false,
  },
  server: {
    port: 5173,
  },
});
