import { defineConfig } from 'vitest/config';

/**
 * Separate from vite.config.ts, which is written for `tauri dev` (a fixed
 * port, an async factory). A test run needs none of it.
 */
export default defineConfig({
  test: {
    environment: 'happy-dom',
    include: ['src/**/*.test.ts', 'src/**/*.test.tsx'],
  },
});
