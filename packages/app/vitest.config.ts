import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    // Node by default: a test that needs `document` or `window` says so on its first line with
    // `// @vitest-environment jsdom`. Building a jsdom window for every file cost about 40 % of a full run
    // (see docs/DEV-SPEED.md); most of the 470 files never touch the DOM.
    environment: 'node',
    // vmThreads runs each file in a V8 context inside a worker thread: no process spawn and a much cheaper jsdom window per
    // file (about 12 s -> 7 s wall on 16 cores, passed 6/6 runs). Its known cost is memory growth on very large suites; if a test
    // ever misbehaves only here, run it with `npm run test:safe` (forks).
    pool: 'vmThreads',
    css: false,
    include: ['src/**/*.test.{ts,tsx}', 'vite-bridge-proxy.test.ts'],
  },
});
