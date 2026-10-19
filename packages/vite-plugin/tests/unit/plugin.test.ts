import { describe, it, expect, vi, beforeEach } from 'vitest'

// Mock BEFORE importing plugin — vi.mock is hoisted by Vitest
vi.mock('caliper-docgen-napi', () => ({
  createSession: vi.fn().mockReturnValue(42),
  initializeSession: vi
    .fn()
    .mockResolvedValue(JSON.stringify({ components: {}, enums: {}, diagnostics: [], stats: {} })),
  extractFileIncremental: vi.fn().mockResolvedValue(
    JSON.stringify({
      updatedComponents: [
        {
          displayName: 'Button',
          filePath: '/project/src/Button.tsx',
          props: {},
          inheritance: [],
          notableInherited: {},
          description: '',
          discriminantProp: null,
          composes: [],
          tags: {},
          methods: [],
        },
      ],
      affectedFiles: ['/project/src/Button.tsx'],
      diagnostics: [],
      durationMs: 5,
    }),
  ),
  closeSession: vi.fn(),
}))

import { caliperDocgen } from '../../src/index.js'
import * as napi from 'caliper-docgen-napi'

describe('caliperDocgen', () => {
  let plugin: ReturnType<typeof caliperDocgen>

  beforeEach(() => {
    vi.clearAllMocks()
    plugin = caliperDocgen({ srcDirs: ['src'] })
    // Simulate Vite calling configResolved
    ;(plugin.configResolved as Function)({ root: '/project' })
  })

  // ── resolveId ──────────────────────────────────────────────────────────────

  describe('resolveId', () => {
    it('resolves the virtual module id', () => {
      const result = (plugin.resolveId as Function)('virtual:caliper-docgen')
      expect(result).toBe('\0virtual:caliper-docgen')
    })

    it('returns undefined for unrelated ids', () => {
      expect((plugin.resolveId as Function)('react')).toBeUndefined()
      expect((plugin.resolveId as Function)('./Button.tsx')).toBeUndefined()
    })
  })

  // ── load ───────────────────────────────────────────────────────────────────

  describe('load', () => {
    it('returns a JS module for the resolved virtual id', () => {
      const code = (plugin.load as Function)('\0virtual:caliper-docgen') as string
      expect(code).toMatch(/^export default \{/)
      const data = JSON.parse(code.replace('export default ', ''))
      expect(data).toHaveProperty('components')
    })

    it('returns undefined for other ids', () => {
      expect((plugin.load as Function)('\0other')).toBeUndefined()
      expect((plugin.load as Function)('virtual:caliper-docgen')).toBeUndefined()
    })
  })

  // ── configResolved ─────────────────────────────────────────────────────────

  describe('configResolved', () => {
    it('creates a NAPI session with resolved absolute src dirs', () => {
      expect(napi.createSession).toHaveBeenCalledWith(expect.objectContaining({ srcDirs: ['/project/src'] }))
    })

    // Unlike buildStart/coldExtract, createSession runs synchronously inside
    // configResolved with no try/catch anywhere in the plugin. Per the Rust
    // binding's own contract this can legitimately throw (a bad reactVersion
    // value, malformed option JSON reaching the NAPI boundary) — it must
    // surface as a clear, prefixed error rather than a raw native stack
    // trace or a silently-undefined sessionId that breaks every later hook.

    it('logs and rethrows when createSession throws', () => {
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
      const boom = new Error('invalid reactVersion')
      ;(napi.createSession as any).mockImplementationOnce(() => {
        throw boom
      })
      const p = caliperDocgen({ srcDirs: ['src'] })
      expect(() => (p.configResolved as Function)({ root: '/project' })).toThrow(boom)
      expect(consoleError).toHaveBeenCalledWith('[caliper-docgen] createSession failed:', boom)
      consoleError.mockRestore()
    })
  })

  // ── configureServer ────────────────────────────────────────────────────────

  describe('configureServer', () => {
    it('calls initializeSession with the session id and resolved options', async () => {
      const mockServer = {
        environments: { client: { hot: { send: vi.fn() } } },
      } as any
      // configureServer returns a teardown fn; initializeSession is async internally.
      // Flush the microtask queue so the .then() callback runs before asserting.
      ;(plugin.configureServer as Function)(mockServer)
      await vi.waitUntil(() => (napi.initializeSession as any).mock.calls.length > 0)
      await (napi.initializeSession as any).mock.results[0].value
      expect(napi.initializeSession).toHaveBeenCalledWith(42, expect.objectContaining({ srcDirs: ['/project/src'] }))
    })

    // Regression test for the dev-server session leak fixed in commit
    // 49b12c6 — buildEnd isn't called in dev mode, so configureServer's
    // returned teardown function is the only reliable cleanup hook. No test
    // previously called the returned function at all.

    it('returns a teardown function that closes the NAPI session', async () => {
      const mockServer = {
        environments: { client: { hot: { send: vi.fn() } } },
      } as any
      const teardown = (plugin.configureServer as Function)(mockServer)
      await vi.waitUntil(() => (napi.initializeSession as any).mock.calls.length > 0)
      await (napi.initializeSession as any).mock.results[0].value

      expect(napi.closeSession).not.toHaveBeenCalled()
      teardown()
      expect(napi.closeSession).toHaveBeenCalledWith(42)
    })
  })

  // ── hotUpdate ──────────────────────────────────────────────────────────────

  describe('hotUpdate', () => {
    const clientEnv = () => ({
      environment: { name: 'client', hot: { send: vi.fn() } },
    })

    const makeOpts = (file: string) => ({
      file,
      type: 'update' as const,
      modules: [],
      timestamp: 1000,
      read: vi.fn().mockResolvedValue(''),
      server: {} as any,
    })

    it('calls extractFileIncremental for tsx files in srcDirs', async () => {
      const ctx = clientEnv()
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      expect(napi.extractFileIncremental).toHaveBeenCalledWith(
        '/project/src/Button.tsx',
        42,
        expect.objectContaining({ srcDirs: ['/project/src'] }),
      )
    })

    it('sends caliper-docgen:update over HMR', async () => {
      const ctx = clientEnv()
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      expect(ctx.environment.hot.send).toHaveBeenCalledWith(
        'caliper-docgen:update',
        expect.objectContaining({ file: '/project/src/Button.tsx' }),
      )
    })

    it('returns undefined so React Fast Refresh still processes the file', async () => {
      const ctx = clientEnv()
      const result = await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      expect(result).toBeUndefined()
    })

    it('skips SSR and other non-client environments', async () => {
      const ctx = { environment: { name: 'ssr', hot: { send: vi.fn() } } }
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      expect(napi.extractFileIncremental).not.toHaveBeenCalled()
    })

    it('skips files outside srcDirs', async () => {
      const ctx = clientEnv()
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/node_modules/react/index.js'))
      expect(napi.extractFileIncremental).not.toHaveBeenCalled()
    })

    it('skips non-ts/tsx files inside srcDirs', async () => {
      const ctx = clientEnv()
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/styles.css'))
      expect(napi.extractFileIncremental).not.toHaveBeenCalled()
    })

    // `.d.ts` files match isSrcFile's `.ts` suffix check and are treated as
    // a valid extraction target — intentional, not an oversight: the core
    // Rust extractor treats declaration files as valid component/interface
    // sources too (`is_tsx = source_type.is_jsx() || is_typescript_definition()`),
    // since a project's own ambient .d.ts declarations can affect prop
    // resolution for components declared elsewhere.

    it('treats a .d.ts file inside srcDirs as a valid extraction target', async () => {
      const ctx = clientEnv()
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/types.d.ts'))
      expect(napi.extractFileIncremental).toHaveBeenCalledWith(
        '/project/src/types.d.ts',
        42,
        expect.objectContaining({ srcDirs: ['/project/src'] }),
      )
    })

    it('skips files in a sibling directory that shares the srcDir prefix', async () => {
      const ctx = clientEnv()
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src-generated/Button.tsx'))
      expect(napi.extractFileIncremental).not.toHaveBeenCalled()
    })

    // A trailing slash on a configured srcDir (relative `'src/'`, or an
    // already-absolute dir ending in `/`) must not silently exclude every
    // file under it — isSrcFile's `startsWith(`${dir}/`)` check would
    // otherwise require a double slash no real file path ever has.

    it('still matches files when srcDirs has a trailing slash (relative)', async () => {
      const p = caliperDocgen({ srcDirs: ['src/'] })
      ;(p.configResolved as Function)({ root: '/project' })
      const ctx = clientEnv()
      await (p.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      expect(napi.extractFileIncremental).toHaveBeenCalledWith(
        '/project/src/Button.tsx',
        42,
        expect.objectContaining({ srcDirs: ['/project/src'] }),
      )
    })

    it('still matches files when srcDirs has a trailing slash (absolute)', async () => {
      const p = caliperDocgen({ srcDirs: ['/project/src/'] })
      ;(p.configResolved as Function)({ root: '/project' })
      const ctx = clientEnv()
      await (p.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      expect(napi.extractFileIncremental).toHaveBeenCalledWith(
        '/project/src/Button.tsx',
        42,
        expect.objectContaining({ srcDirs: ['/project/src'] }),
      )
    })

    // The documented fallback contract (src/index.ts) is that clients still
    // receive an update event rather than an unhandled rejection when the
    // NAPI call resolves with unparsable JSON — never exercised before now.

    it('sends an empty update and logs when extractFileIncremental returns malformed JSON', async () => {
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
      ;(napi.extractFileIncremental as any).mockResolvedValueOnce('not valid json')
      const ctx = clientEnv()
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))

      expect(ctx.environment.hot.send).toHaveBeenCalledWith('caliper-docgen:update', {
        file: '/project/src/Button.tsx',
        updatedComponents: [],
        diagnostics: [],
      })
      expect(consoleError).toHaveBeenCalledWith(
        '[caliper-docgen] Failed to parse extractFileIncremental output:',
        expect.any(Error),
      )
      consoleError.mockRestore()
    })

    // The queue/replay race-guard described in the archived plugin spec: a
    // file-change event arriving while cold extraction is still in flight
    // must wait for it, not race it. Every other hotUpdate test calls
    // hotUpdate directly without configureServer, so initPromise is always
    // null and this blocking behavior was never actually exercised.

    it('waits for a still-pending initPromise before calling extractFileIncremental', async () => {
      let resolveInit: (value: string) => void = () => {}
      const initDeferred = new Promise<string>((resolve) => {
        resolveInit = resolve
      })
      ;(napi.initializeSession as any).mockReturnValueOnce(initDeferred)

      const mockServer = { environments: { client: { hot: { send: vi.fn() } } } } as any
      ;(plugin.configureServer as Function)(mockServer)
      // Cold extraction is now in flight — initPromise is set and unresolved.

      const ctx = clientEnv()
      const hotUpdatePromise = (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))

      await Promise.resolve()
      await Promise.resolve()
      expect(napi.extractFileIncremental).not.toHaveBeenCalled()

      resolveInit(JSON.stringify({ components: {}, enums: {}, diagnostics: [], stats: {} }))
      await hotUpdatePromise

      expect(napi.extractFileIncremental).toHaveBeenCalled()
    })

    // Two overlapping hotUpdate calls for the same file (e.g. a rapid
    // double-save) have no sequencing guard against their async NAPI calls
    // resolving out of order — an older, now-stale result could silently
    // overwrite a newer one already applied.

    const makeComponent = (tag: string) => ({
      displayName: 'Button',
      filePath: '/project/src/Button.tsx',
      props: {},
      inheritance: [],
      notableInherited: {},
      description: tag,
      discriminantProp: null,
      composes: [],
      tags: {},
      methods: [],
    })

    it('drops a stale result when an older hotUpdate call for the same file resolves after a newer one', async () => {
      let resolveFirst: (v: string) => void = () => {}
      let resolveSecond: (v: string) => void = () => {}
      const firstDeferred = new Promise<string>((r) => {
        resolveFirst = r
      })
      const secondDeferred = new Promise<string>((r) => {
        resolveSecond = r
      })
      ;(napi.extractFileIncremental as any).mockReturnValueOnce(firstDeferred).mockReturnValueOnce(secondDeferred)

      const ctx = clientEnv()
      const firstCall = (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      await Promise.resolve()
      const secondCall = (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      await Promise.resolve()

      // Second (newer) call's NAPI promise resolves FIRST — out of order.
      resolveSecond(JSON.stringify({ updatedComponents: [makeComponent('fresh')], diagnostics: [] }))
      await secondCall

      // First (older) call resolves LAST — its result must be dropped, not
      // applied on top of the newer one.
      resolveFirst(JSON.stringify({ updatedComponents: [makeComponent('stale')], diagnostics: [] }))
      await firstCall

      const code = (plugin.load as Function)('\0virtual:caliper-docgen') as string
      const data = JSON.parse(code.replace('export default ', ''))
      expect(data.components.Button.description).toBe('fresh')
    })

    it('does not let the staleness guard for one file drop a concurrent update for a different file', async () => {
      let resolveButton: (v: string) => void = () => {}
      let resolveInput: (v: string) => void = () => {}
      const buttonDeferred = new Promise<string>((r) => {
        resolveButton = r
      })
      const inputDeferred = new Promise<string>((r) => {
        resolveInput = r
      })
      ;(napi.extractFileIncremental as any).mockReturnValueOnce(buttonDeferred).mockReturnValueOnce(inputDeferred)

      const ctx = clientEnv()
      const buttonCall = (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      await Promise.resolve()
      const inputCall = (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Input.tsx'))
      await Promise.resolve()

      resolveInput(
        JSON.stringify({
          updatedComponents: [{ ...makeComponent('input'), displayName: 'Input', filePath: '/project/src/Input.tsx' }],
          diagnostics: [],
        }),
      )
      await inputCall
      resolveButton(JSON.stringify({ updatedComponents: [makeComponent('button')], diagnostics: [] }))
      await buttonCall

      const code = (plugin.load as Function)('\0virtual:caliper-docgen') as string
      const data = JSON.parse(code.replace('export default ', ''))
      expect(data.components.Button.description).toBe('button')
      expect(data.components.Input.description).toBe('input')
    })
  })

  // ── buildEnd ───────────────────────────────────────────────────────────────

  describe('buildEnd', () => {
    it('closes the NAPI session', () => {
      ;(plugin.buildEnd as Function)()
      expect(napi.closeSession).toHaveBeenCalledWith(42)
    })

    it('still closes the session for a normal (non-watch) vite build', () => {
      const p = caliperDocgen({ srcDirs: ['src'] })
      ;(p.configResolved as Function)({ root: '/project', command: 'build' })
      ;(p.buildEnd as Function)()
      expect(napi.closeSession).toHaveBeenCalledWith(42)
    })
  })

  // ── buildEnd / closeWatcher under `vite build --watch` ────────────────────
  //
  // Per Rollup's own plugin hook contract, buildEnd fires once per rebuild
  // in watch mode — not once when the watch process actually shuts down
  // (that's what closeWatcher is for). Closing the session in buildEnd
  // during a watched build would make every subsequent rebuild's buildStart
  // auto-vivify a brand new session (the Rust binding's initializeSession
  // creates one for any unrecognized/closed session id), silently losing
  // all incremental-cache benefit on every rebuild after the first.

  describe('buildEnd / closeWatcher session lifecycle under `vite build --watch`', () => {
    it('does not close the session on buildEnd while watching', () => {
      const p = caliperDocgen({ srcDirs: ['src'] })
      ;(p.configResolved as Function)({ root: '/project', command: 'build', build: { watch: {} } })
      ;(p.buildEnd as Function)()
      ;(p.buildEnd as Function)() // a second rebuild's buildEnd
      expect(napi.closeSession).not.toHaveBeenCalled()
    })

    it('closes the session via closeWatcher when the watch process shuts down', () => {
      const p = caliperDocgen({ srcDirs: ['src'] })
      ;(p.configResolved as Function)({ root: '/project', command: 'build', build: { watch: {} } })
      ;(p.closeWatcher as Function)()
      expect(napi.closeSession).toHaveBeenCalledWith(42)
    })
  })

  // ── buildStart ─────────────────────────────────────────────────────────────
  //
  // configureServer (which runs the cold extraction that populates the virtual
  // module's data) is a dev-server-only hook — Vite never calls it during
  // `vite build`. Without a build-mode extraction path, a production build's
  // `virtual:caliper-docgen` import always resolved to the empty placeholder
  // stats/components declared at plugin construction time.

  describe('buildStart', () => {
    it('runs cold extraction when the command is "build"', async () => {
      ;(plugin.configResolved as Function)({ root: '/project', command: 'build' })
      await (plugin.buildStart as Function)()
      expect(napi.initializeSession).toHaveBeenCalledWith(42, expect.objectContaining({ srcDirs: ['/project/src'] }))
      const code = (plugin.load as Function)('\0virtual:caliper-docgen') as string
      expect(code).toMatch(/^export default \{/)
    })

    it('does not run cold extraction when the command is "serve" (configureServer handles it)', async () => {
      ;(plugin.configResolved as Function)({ root: '/project', command: 'serve' })
      await (plugin.buildStart as Function)()
      expect(napi.initializeSession).not.toHaveBeenCalled()
    })

    // coldExtract's own malformed-JSON fallback (distinct from hotUpdate's —
    // see that describe block): logs and leaves currentOutput at its
    // placeholder value rather than throwing out of buildStart.

    it('logs and leaves the virtual module at its placeholder when initializeSession returns malformed JSON', async () => {
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
      ;(napi.initializeSession as any).mockResolvedValueOnce('not valid json')
      ;(plugin.configResolved as Function)({ root: '/project', command: 'build' })
      await (plugin.buildStart as Function)()

      expect(consoleError).toHaveBeenCalledWith(
        '[caliper-docgen] Failed to parse initializeSession output:',
        expect.any(Error),
      )
      const code = (plugin.load as Function)('\0virtual:caliper-docgen') as string
      const data = JSON.parse(code.replace('export default ', ''))
      expect(data.components).toEqual({})
      consoleError.mockRestore()
    })
  })

  // ── skipHtmlProps ──────────────────────────────────────────────────────────

  describe('skipHtmlProps', () => {
    it('maps to htmlAttributes: "none" for the NAPI call', () => {
      const p = caliperDocgen({ srcDirs: ['src'], skipHtmlProps: true })
      ;(p.configResolved as Function)({ root: '/project' })
      expect(napi.createSession).toHaveBeenCalledWith(expect.objectContaining({ htmlAttributes: 'none' }))
    })

    it('leaves htmlAttributes unset when skipHtmlProps is not passed', () => {
      const p = caliperDocgen({ srcDirs: ['src'] })
      ;(p.configResolved as Function)({ root: '/project' })
      expect(napi.createSession).toHaveBeenCalledWith(expect.objectContaining({ htmlAttributes: undefined }))
    })
  })

  // ── exclude / reactVersion pass-through ────────────────────────────────────
  //
  // Simple pass-through in napiOptions(), but previously unasserted by any
  // test with a non-default value.

  describe('exclude / reactVersion options forwarding', () => {
    it('forwards exclude and reactVersion to the NAPI call', () => {
      const p = caliperDocgen({ srcDirs: ['src'], exclude: ['**/*.stories.tsx'], reactVersion: 'react19' })
      ;(p.configResolved as Function)({ root: '/project' })
      expect(napi.createSession).toHaveBeenCalledWith(
        expect.objectContaining({ exclude: ['**/*.stories.tsx'], reactVersion: 'react19' }),
      )
    })

    it('leaves exclude and reactVersion unset when not passed', () => {
      const p = caliperDocgen({ srcDirs: ['src'] })
      ;(p.configResolved as Function)({ root: '/project' })
      expect(napi.createSession).toHaveBeenCalledWith(
        expect.objectContaining({ exclude: undefined, reactVersion: undefined }),
      )
    })
  })
})
