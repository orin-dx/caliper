// Layer 2 — integration tests, against the REAL @orin-dx/caliper-docgen-napi native
// binding (never mocked here — see tests/unit/plugin.test.ts for the mocked
// unit tier). Requires a locally-built `.node` binary
// (`pnpm --filter @orin-dx/caliper-docgen-napi build:napi`); auto-skips the whole
// file when one isn't available, so a fresh checkout without the native
// toolchain doesn't fail. Set NAPI_BINARY_AVAILABLE=false to force-skip
// regardless of whether a binary happens to be present.
//
// Run via `pnpm test:integration` (a separate vitest config, pool: 'forks' —
// required, the native binary cannot load inside worker threads).

import { describe, it, expect, afterEach } from 'vitest'
import { createServer, type ViteDevServer } from 'vite'
import * as fs from 'node:fs'
import * as path from 'node:path'
import { fileURLToPath } from 'node:url'
import { caliperDocgen } from '../../src/index.js'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const FIXTURES_ROOT = path.join(__dirname, 'fixtures')
const BUTTON_FIXTURE = path.join(FIXTURES_ROOT, 'src/Button.tsx')

let napiAvailable = false
try {
  await import('@orin-dx/caliper-docgen-napi')
  napiAvailable = true
} catch {
  napiAvailable = false
}

const shouldRun = napiAvailable && process.env.NAPI_BINARY_AVAILABLE !== 'false'

// Vite's SSR module graph caches a transformed module by id — a plain
// repeated ssrLoadModule call keeps returning the FIRST (often still-empty,
// pre-cold-extraction) result forever, never re-invoking the plugin's load()
// hook to pick up currentOutput's latest state. invalidateAll() forces a
// fresh transform on every call, which is what makes polling for the real
// async cold-extraction result actually work.
async function loadVirtualModule(server: ViteDevServer): Promise<any> {
  server.moduleGraph.invalidateAll()
  const mod = await server.ssrLoadModule('virtual:caliper-docgen')
  return mod.default
}

async function waitForComponents(server: ViteDevServer, timeoutMs = 5000): Promise<any> {
  const start = Date.now()
  while (Date.now() - start < timeoutMs) {
    const data = await loadVirtualModule(server)
    if (Object.keys(data.components).length > 0) return data
    await new Promise((resolve) => setTimeout(resolve, 20))
  }
  throw new Error(`waitForComponents: no components populated within ${timeoutMs}ms`)
}

describe.skipIf(!shouldRun)('caliperDocgen (integration, real native binding)', () => {
  let server: ViteDevServer | undefined
  const originalButtonSource = fs.readFileSync(BUTTON_FIXTURE, 'utf8')

  afterEach(async () => {
    // Restore the fixture in case a test mutated it, and always close the
    // server so its session-teardown hooks run before the next test.
    fs.writeFileSync(BUTTON_FIXTURE, originalButtonSource)
    if (server) {
      await server.close()
      server = undefined
    }
  })

  it('resolves the virtual module with real extracted data for a real fixture file', async () => {
    server = await createServer({
      configFile: false,
      root: FIXTURES_ROOT,
      server: { middlewareMode: true },
      plugins: [caliperDocgen({ srcDirs: ['src'] })],
    })

    const data = await waitForComponents(server)

    expect(data.components).toHaveProperty('Button')
    const button = data.components.Button
    expect(Object.keys(button.props).toSorted()).toEqual(['label', 'onClick', 'variant'])
    expect(button.props.label.required).toBe(true)
    expect(button.props.variant.required).toBe(false)
    // The NAPI boundary's ParsedProp.defaultValue.value carries the prop's
    // source-text literal verbatim, quotes included — not a bare JS string.
    expect(button.props.variant.defaultValue?.value).toBe('"primary"')
  })

  it('re-extracts real prop changes via a direct hotUpdate call against the real binding', async () => {
    const plugin = caliperDocgen({ srcDirs: ['src'] })
    server = await createServer({
      configFile: false,
      root: FIXTURES_ROOT,
      server: { middlewareMode: true },
      plugins: [plugin],
    })

    await waitForComponents(server)

    // Real edit to the real fixture file — a genuinely new prop.
    fs.writeFileSync(
      BUTTON_FIXTURE,
      `export interface ButtonProps {
  label: string
  disabled?: boolean
}
export function Button({ label, disabled = false }: ButtonProps) {
  return { label, disabled }
}
`,
    )

    const hotSend: Array<{ event: string; data: unknown }> = []
    const ctx = {
      environment: {
        name: 'client',
        hot: { send: (event: string, data: unknown) => hotSend.push({ event, data }) },
      },
    }
    await (plugin.hotUpdate as any).call(ctx, {
      file: BUTTON_FIXTURE,
      type: 'update',
      modules: [],
      timestamp: Date.now(),
      read: async () => fs.readFileSync(BUTTON_FIXTURE, 'utf8'),
      server,
    })

    const updateEvent = hotSend.find((e) => e.event === 'caliper-docgen:update')
    expect(updateEvent).toBeDefined()
    const updated = (updateEvent!.data as any).updatedComponents[0]
    expect(updated.displayName).toBe('Button')
    expect(Object.keys(updated.props).toSorted()).toEqual(['disabled', 'label'])

    const data = await loadVirtualModule(server)
    expect(Object.keys(data.components.Button.props).toSorted()).toEqual(['disabled', 'label'])
  })

  it('closes the real NAPI session when the server closes', async () => {
    server = await createServer({
      configFile: false,
      root: FIXTURES_ROOT,
      server: { middlewareMode: true },
      plugins: [caliperDocgen({ srcDirs: ['src'] })],
    })
    await waitForComponents(server)

    // No thrown error/hang on close is the observable contract here — the
    // teardown function returned by configureServer (which calls the real
    // napi.closeSession) is wired into Vite's own close lifecycle.
    await expect(server.close()).resolves.toBeUndefined()
    server = undefined
  })
})
