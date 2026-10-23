// Layer 2 — integration tests, against the REAL caliper-docgen-napi native
// binding (never mocked here — see tests/unit/plugin.test.ts for the mocked
// unit tier). Requires a locally-built `.node` binary
// (`pnpm --filter caliper-docgen-napi build:napi`); auto-skips the whole
// file when one isn't available, so a fresh checkout without the native
// toolchain doesn't fail. Set NAPI_BINARY_AVAILABLE=false to force-skip
// regardless of whether a binary happens to be present.
//
// Run via `pnpm test:integration` (a separate vitest config, pool: 'forks').

import { describe, it, expect, afterEach } from 'vitest'
import { build, createServer, type ViteDevServer } from 'vite'
import * as fs from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import { fileURLToPath } from 'node:url'
import { caliperDocgen } from '../../src/index.js'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const FIXTURES_ROOT = path.join(__dirname, 'fixtures')
const BUTTON_FIXTURE = path.join(FIXTURES_ROOT, 'src/Button.tsx')

let napiAvailable = false
try {
  await import('caliper-docgen-napi')
  napiAvailable = true
} catch {
  napiAvailable = false
}

// CI builds the binary and sets this, so a missing or unloadable binary fails instead of skipping.
if (!napiAvailable && process.env.NAPI_BINARY_REQUIRED === 'true') {
  throw new Error('caliper-docgen-napi did not load; build it with `pnpm --filter caliper-docgen-napi build:napi`')
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

/** A throwaway project holding `files`; its real path, since the native side reports canonical paths. */
function tempProject(files: Record<string, string>): string {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'caliper-vite-')))
  for (const [name, body] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true })
    fs.writeFileSync(path.join(root, name), body)
  }
  return root
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
    const prop = (name: string) => data.props[button.props[name]]
    expect(prop('label').required).toBe(true)
    expect(prop('variant').required).toBe(false)
    // The NAPI boundary's ParsedProp.defaultValue.value carries the prop's
    // source-text literal verbatim, quotes included — not a bare JS string.
    expect(prop('variant').defaultValue?.value).toBe('"primary"')
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
  return <button disabled={disabled}>{label}</button>
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
    const payload = updateEvent!.data as any
    const updated = payload.updatedComponents[0]
    expect(updated.displayName).toBe('Button')
    expect(Object.keys(updated.props).toSorted()).toEqual(['disabled', 'label'])
    expect(payload.props[updated.props.disabled].defaultValue?.value).toBe('false')

    const data = await loadVirtualModule(server)
    expect(Object.keys(data.components.Button.props).toSorted()).toEqual(['disabled', 'label'])
  })

  it('serves dependency edits and deletions through the virtual module and the update event', async () => {
    const root = tempProject({
      'src/types.ts': 'export interface ButtonProps { label: string }\n',
      'src/Button.tsx':
        "import type { ButtonProps } from './types'\nexport function Button(props: ButtonProps) { return null }\n",
      'src/Card.tsx': 'export function Card(props: { title: string }) { return null }\n',
    })
    const plugin = caliperDocgen({ srcDirs: ['src'] })
    server = await createServer({ configFile: false, root, server: { middlewareMode: true }, plugins: [plugin] })
    await waitForComponents(server)
    const client = server.environments.client
    const sent: any[] = []
    const ctx = {
      environment: {
        name: 'client',
        hot: { send: (event: string, data: unknown) => event === 'caliper-docgen:update' && sent.push(data) },
        moduleGraph: client.moduleGraph,
      },
    }
    const change = (file: string) =>
      (plugin.hotUpdate as any).call(ctx, { file, type: 'update', modules: [], timestamp: Date.now(), server })

    fs.writeFileSync(path.join(root, 'src/types.ts'), 'export interface ButtonProps { label: string; size?: number }\n')
    await change(path.join(root, 'src/types.ts'))
    fs.rmSync(path.join(root, 'src/Card.tsx'))
    await change(path.join(root, 'src/Card.tsx'))

    expect(sent.map(({ updatedKeys, removedKeys }) => ({ updatedKeys, removedKeys }))).toEqual([
      { updatedKeys: ['Button'], removedKeys: [] },
      { updatedKeys: [], removedKeys: ['Card'] },
    ])
    const data = await loadVirtualModule(server)
    expect(Object.keys(data.components)).toEqual(['Button'])
    expect(Object.keys(data.components.Button.props).toSorted()).toEqual(['label', 'size'])
  })

  it('bundles the extracted output into a production build', async () => {
    const root = tempProject({
      'src/Button.tsx': 'export function Button(props: { label: string }) { return null }\n',
      'index.js': "import docs from 'virtual:caliper-docgen'\nexport default docs\n",
    })

    const result = (await build({
      configFile: false,
      root,
      logLevel: 'silent',
      build: { write: false, lib: { entry: path.join(root, 'index.js'), formats: ['es'], fileName: 'docs' } },
      plugins: [caliperDocgen({ srcDirs: ['src'] })],
    })) as any

    const code: string = result[0].output[0].code
    expect(code).toContain('displayName: "Button"')
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
