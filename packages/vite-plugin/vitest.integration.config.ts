import { defineConfig } from 'vitest/config'

// Layer 2 (see tests/integration/plugin.integration.test.ts's own header
// comment). pool: 'forks' keeps each test file's native watch sessions in its
// own process; the binding itself loads in worker threads too
// (napi-worker-threads.integration.test.ts).
export default defineConfig({
  test: {
    pool: 'forks',
    include: ['tests/integration/**/*.test.ts'],
    testTimeout: 15000,
  },
})
