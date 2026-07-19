import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { viteSingleFile } from 'vite-plugin-singlefile';

// Builds back to a single self-contained tools/theme-editor/index.html —
// the user manual links directly to that file on GitHub as a standalone download.
export default defineConfig({
  root: 'src',
  plugins: [react(), viteSingleFile()],
  build: {
    outDir: '..',
    emptyOutDir: false,
  },
});
