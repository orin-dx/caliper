// The native binding inside `node:worker_threads`, where Storybook 11 runs docgen providers: each worker loads the
// addon, extracts, and runs its own watch session, several at once and again after earlier workers have exited.

import { describe, it, expect } from 'vitest'
import { Worker } from 'node:worker_threads'
import * as path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const PROBE = path.join(__dirname, 'fixtures/worker-probe.mjs')
const SRC = path.join(__dirname, 'fixtures/src')

let napiAvailable = false
try {
  await import('caliper-docgen-napi')
  napiAvailable = true
} catch {
  napiAvailable = false
}
if (!napiAvailable && process.env.NAPI_BINARY_REQUIRED === 'true') {
  throw new Error('caliper-docgen-napi did not load; build it with `pnpm --filter caliper-docgen-napi build:napi`')
}

interface ProbeResult {
  cold: string[]
  session: string[]
  sessionId: number
}

function runProbe(): Promise<ProbeResult> {
  return new Promise((resolve, reject) => {
    const worker = new Worker(PROBE, { workerData: { srcDir: SRC } })
    worker.once('message', resolve)
    worker.once('error', reject)
    worker.once('exit', (code) => {
      if (code !== 0) reject(new Error(`worker exited with ${code}`))
    })
  })
}

describe.skipIf(!napiAvailable)('caliper-docgen-napi in worker threads', () => {
  it('extracts and runs a session in several workers at once, and again after they exit', async () => {
    const concurrent = await Promise.all([runProbe(), runProbe(), runProbe()])
    for (const result of concurrent) {
      expect(result.cold).toEqual(['Button'])
      expect(result.session).toEqual(['Button'])
    }
    expect(new Set(concurrent.map((r) => r.sessionId)).size).toBe(3)

    const later = await runProbe()
    expect(later.cold).toEqual(['Button'])
    expect(later.session).toEqual(['Button'])
  })
})
