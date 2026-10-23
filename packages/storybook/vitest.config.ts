import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    // The provider resolves story paths against the working directory, which only a forked worker may change.
    pool: 'forks',
    include: ['tests/unit/**/*.test.ts'],
    coverage: { provider: 'v8', include: ['src/**/*.ts'], reporter: ['text', 'lcov'] },
  },
})
