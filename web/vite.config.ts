import { defineConfig } from 'vite';
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';

// base '/' because the site is served at the apex of talk.pilgrimapp.org (a
// custom domain), not a project subpath. Deep-links use the hash fragment, so
// they never hit the server and never 404 on Pages.
export default defineConfig({
  base: '/',
  plugins: [wasm(), topLevelAwait()],
  build: {
    target: 'esnext',
    outDir: 'dist',
  },
});
