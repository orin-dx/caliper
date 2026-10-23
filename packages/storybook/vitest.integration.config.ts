import { defineConfig } from 'vitest/config'

// pool: 'forks': the tests change the working directory, which the provider's worker thread shares.
export default defineConfig({
  test: {
    pool: 'forks',
    include: ['tests/integration/**/*.test.ts'],
    testTimeout: 15000,
  },
})
