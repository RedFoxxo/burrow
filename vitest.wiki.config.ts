import { defineConfig } from 'vitest/config'

// Every tool against a real LeafWiki (the pinned release in tests/wiki/leafwiki.ts),
// started on a free local port with throwaway data. Never part of `npm test`:
// the first run downloads the binary.
export default defineConfig({
  test: {
    environment: 'node',
    include: ['tests/wiki/**/*.test.ts'],
    globalSetup: ['tests/wiki/setup.ts'],
    testTimeout: 30_000,
    hookTimeout: 120_000,
    fileParallelism: false,
  },
})
