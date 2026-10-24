import { describe, it, expect, vi, beforeEach } from 'vitest'

const SNAPSHOT = JSON.stringify({
  schemaVersion: 2,
  components: { Button: { displayName: 'Button', filePath: '/project/src/Button.tsx' } },
  enums: {},
  diagnostics: [],
  stats: {},
})

const component = (displayName: string, description = '') => ({
  displayName,
  filePath: `/project/src/${displayName}.tsx`,
  props: {},
  inheritance: [],
  notableInherited: {},
  description,
  discriminantProp: null,
  composes: [],
  tags: {},
  methods: [],
})

const update = (revision: number, updated: string[], removed: string[] = [], description = '') =>
  JSON.stringify({
    schemaVersion: 2,
    revision,
    updatedComponents: updated.map((name) => component(name, description)),
    props: {},
    propSets: {},
    nameLists: {},
    updatedKeys: updated,
    removedKeys: removed,
    affectedFiles: [],
    diagnostics: [],
    removedDiagnostics: [],
    durationMs: 1,
  })

// Mock BEFORE importing plugin — vi.mock is hoisted by Vitest
vi.mock('caliper-docgen-napi', () => ({
  createSession: vi.fn().mockReturnValue(42),
  initializeSession: vi.fn(),
  extractFileIncremental: vi.fn(),
  closeSession: vi.fn(),
  sessionDocgenInfo: vi.fn(),
  resolutionConfigFiles: vi.fn(() => ['/project/tsconfig.json']),
  isResolutionConfig: vi.fn((_session: number, file: string) => file === '/project/tsconfig.json'),
}))

import { caliperDocgen } from '../../src/index.js'
import * as napi from 'caliper-docgen-napi'

/** A Rollup/Vite plugin context whose `error` throws like the real one. */
const pluginContext = () => ({
  error: vi.fn((message: string) => {
    throw new Error(message)
  }),
  addWatchFile: vi.fn(),
})

/** A dev server whose client environment sends HMR events through `send`. */
const server = ({ send }: { send: (event: string, data?: unknown) => void }) => ({
  environments: { client: { hot: { send } } },
  watcher: { add: vi.fn() },
})

describe('caliperDocgen', () => {
  let plugin: ReturnType<typeof caliperDocgen>

  beforeEach(() => {
    vi.clearAllMocks()
    ;(napi.initializeSession as any).mockResolvedValue(SNAPSHOT)
    ;(napi.extractFileIncremental as any).mockResolvedValue(update(2, ['Button']))
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
    it("serves the session's current output", async () => {
      const code = await (plugin.load as Function).call(pluginContext(), '\0virtual:caliper-docgen')
      expect(code).toBe(`export default ${SNAPSHOT}`)
      expect(napi.initializeSession).toHaveBeenCalledWith(42, expect.objectContaining({ srcDirs: ['/project/src'] }))
    })

    it('returns undefined for other ids', async () => {
      expect(await (plugin.load as Function).call(pluginContext(), '\0other')).toBeUndefined()
      expect(await (plugin.load as Function).call(pluginContext(), 'virtual:caliper-docgen')).toBeUndefined()
    })

    it('fails the module load when extraction fails', async () => {
      ;(napi.initializeSession as any).mockRejectedValueOnce(new Error('boom'))
      const ctx = pluginContext()
      await expect((plugin.load as Function).call(ctx, '\0virtual:caliper-docgen')).rejects.toThrow(
        '[caliper-docgen] extraction failed: Error: boom',
      )
    })
  })

  // ── configResolved ─────────────────────────────────────────────────────────

  describe('configResolved', () => {
    it('creates a NAPI session with resolved absolute src dirs', () => {
      expect(napi.createSession).toHaveBeenCalledWith(expect.objectContaining({ srcDirs: ['/project/src'] }))
    })

    // createSession runs synchronously inside configResolved. Per the Rust
    // binding's own contract this can legitimately throw (a bad htmlAttributes
    // value, malformed option JSON reaching the NAPI boundary) — it must
    // surface as a clear, prefixed error rather than a raw native stack
    // trace or a silently-undefined sessionId that breaks every later hook.

    it('logs and rethrows when createSession throws', () => {
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
      const boom = new Error('invalid htmlAttributes')
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
    it('runs the cold extraction and announces it', async () => {
      const send = vi.fn()
      ;(plugin.configureServer as Function)(server({ send }))
      await vi.waitUntil(() => send.mock.calls.length > 0)
      expect(napi.initializeSession).toHaveBeenCalledWith(42, expect.objectContaining({ srcDirs: ['/project/src'] }))
      expect(send).toHaveBeenCalledWith('caliper-docgen:ready', {
        components: { Button: { displayName: 'Button', filePath: '/project/src/Button.tsx' } },
      })
    })

    it('watches the tsconfigs, package.json files and lockfiles that decide how imports resolve', () => {
      const devServer = server({ send: vi.fn() })
      ;(plugin.configureServer as Function)(devServer)
      expect(napi.resolutionConfigFiles).toHaveBeenCalledWith(42)
      expect(devServer.watcher.add).toHaveBeenCalledWith(['/project/tsconfig.json'])
    })

    // buildEnd isn't called in dev mode, so configureServer's returned
    // teardown function is the only reliable cleanup hook.

    it('returns a teardown function that closes the NAPI session', async () => {
      const teardown = (plugin.configureServer as Function)(server({ send: vi.fn() }))
      await vi.waitUntil(() => (napi.initializeSession as any).mock.calls.length > 0)

      expect(napi.closeSession).not.toHaveBeenCalled()
      teardown()
      expect(napi.closeSession).toHaveBeenCalledWith(42)
    })
  })

  // ── hotUpdate ──────────────────────────────────────────────────────────────

  describe('hotUpdate', () => {
    const virtualModule = { id: '\0virtual:caliper-docgen' }

    const clientEnv = (withVirtualModule = false) => ({
      environment: {
        name: 'client',
        hot: { send: vi.fn() },
        moduleGraph: {
          getModuleById: vi.fn((id: string) =>
            withVirtualModule && id === virtualModule.id ? virtualModule : undefined,
          ),
          invalidateModule: vi.fn(),
        },
      },
    })

    const changedModule = { id: '/project/src/Button.tsx' }
    const makeOpts = (file: string) => ({
      file,
      type: 'update' as const,
      modules: [changedModule],
      timestamp: 1000,
      read: vi.fn().mockResolvedValue(''),
      server: {} as any,
    })

    const sentUpdates = (ctx: ReturnType<typeof clientEnv>) =>
      ctx.environment.hot.send.mock.calls.filter(([event]) => event === 'caliper-docgen:update').map(([, data]) => data)

    it('calls extractFileIncremental for tsx files in srcDirs', async () => {
      await (plugin.hotUpdate as Function).call(clientEnv(), makeOpts('/project/src/Button.tsx'))
      expect(napi.extractFileIncremental).toHaveBeenCalledWith(
        '/project/src/Button.tsx',
        42,
        expect.objectContaining({ srcDirs: ['/project/src'] }),
      )
    })

    it('sends the keyed update over HMR', async () => {
      ;(napi.extractFileIncremental as any).mockResolvedValueOnce(update(2, ['Panel'], ['Card']))
      const ctx = clientEnv()
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Card.tsx'))
      expect(sentUpdates(ctx)).toEqual([
        {
          file: '/project/src/Card.tsx',
          revision: 2,
          updatedKeys: ['Panel'],
          updatedComponents: [component('Panel')],
          props: {},
          propSets: {},
          nameLists: {},
          removedKeys: ['Card'],
          diagnostics: [],
          removedDiagnostics: [],
        },
      ])
    })

    it('invalidates the virtual module and returns it with the changed modules', async () => {
      const ctx = clientEnv(true)
      const result = await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      expect(ctx.environment.moduleGraph.invalidateModule).toHaveBeenCalledWith(virtualModule)
      expect(result).toEqual([changedModule, virtualModule])
    })

    it('returns undefined when nothing imports the virtual module, so Fast Refresh handles the file', async () => {
      const result = await (plugin.hotUpdate as Function).call(clientEnv(), makeOpts('/project/src/Button.tsx'))
      expect(result).toBeUndefined()
    })

    it('sends nothing for a change that produced no new revision', async () => {
      ;(napi.extractFileIncremental as any).mockResolvedValueOnce(update(1, []))
      const ctx = clientEnv(true)
      const result = await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.stories.tsx'))
      expect(sentUpdates(ctx)).toEqual([])
      expect(ctx.environment.moduleGraph.invalidateModule).not.toHaveBeenCalled()
      expect(result).toBeUndefined()
    })

    it('still reloads for a change that only changed diagnostics', async () => {
      ;(napi.extractFileIncremental as any).mockResolvedValueOnce(update(2, []))
      const ctx = clientEnv(true)
      const result = await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/types.ts'))
      expect(sentUpdates(ctx)).toHaveLength(1)
      expect(result).toEqual([changedModule, virtualModule])
    })

    it('skips SSR and other non-client environments', async () => {
      const ctx = { environment: { name: 'ssr', hot: { send: vi.fn() } } }
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      expect(napi.extractFileIncremental).not.toHaveBeenCalled()
    })

    it('skips files outside srcDirs', async () => {
      await (plugin.hotUpdate as Function).call(clientEnv(), makeOpts('/project/node_modules/react/index.js'))
      expect(napi.extractFileIncremental).not.toHaveBeenCalled()
    })

    it('skips non-ts/tsx files inside srcDirs', async () => {
      await (plugin.hotUpdate as Function).call(clientEnv(), makeOpts('/project/src/styles.css'))
      expect(napi.extractFileIncremental).not.toHaveBeenCalled()
    })

    // A project's own .d.ts declarations can affect prop resolution for components declared elsewhere.
    it('treats a .d.ts file inside srcDirs as a valid extraction target', async () => {
      await (plugin.hotUpdate as Function).call(clientEnv(), makeOpts('/project/src/types.d.ts'))
      expect(napi.extractFileIncremental).toHaveBeenCalledWith(
        '/project/src/types.d.ts',
        42,
        expect.objectContaining({ srcDirs: ['/project/src'] }),
      )
    })

    it('skips files in a sibling directory that shares the srcDir prefix', async () => {
      await (plugin.hotUpdate as Function).call(clientEnv(), makeOpts('/project/src-generated/Button.tsx'))
      expect(napi.extractFileIncremental).not.toHaveBeenCalled()
    })

    // A trailing slash on a configured srcDir must not silently exclude every file under it.
    it.each([['src/'], ['/project/src/']])('still matches files when srcDirs is %s', async (dir) => {
      const p = caliperDocgen({ srcDirs: [dir] })
      ;(p.configResolved as Function)({ root: '/project' })
      await (p.hotUpdate as Function).call(clientEnv(), makeOpts('/project/src/Button.tsx'))
      expect(napi.extractFileIncremental).toHaveBeenCalledWith(
        '/project/src/Button.tsx',
        42,
        expect.objectContaining({ srcDirs: ['/project/src'] }),
      )
    })

    it('logs and sends nothing when the change cannot be applied', async () => {
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
      ;(napi.extractFileIncremental as any).mockResolvedValueOnce('not valid json')
      const ctx = clientEnv(true)
      const result = await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))

      expect(sentUpdates(ctx)).toEqual([])
      expect(result).toBeUndefined()
      expect(consoleError).toHaveBeenCalledWith('[caliper-docgen] Failed to apply the change:', expect.any(Error))
      consoleError.mockRestore()
    })

    // A file-change event arriving while cold extraction is still in flight must wait for it, not race it.
    it('waits for a still-pending cold extraction before calling extractFileIncremental', async () => {
      let resolveInit: (value: string) => void = () => {}
      ;(napi.initializeSession as any).mockReturnValueOnce(
        new Promise<string>((resolve) => {
          resolveInit = resolve
        }),
      )
      ;(plugin.configureServer as Function)(server({ send: vi.fn() }))

      const hotUpdatePromise = (plugin.hotUpdate as Function).call(clientEnv(), makeOpts('/project/src/Button.tsx'))
      await Promise.resolve()
      await Promise.resolve()
      expect(napi.extractFileIncremental).not.toHaveBeenCalled()

      resolveInit(SNAPSHOT)
      await hotUpdatePromise
      expect(napi.extractFileIncremental).toHaveBeenCalled()
    })

    // Each update is a delta from the revision before it, so a second change, to any file, starts only after the first
    // has been applied and announced.
    it('applies and announces changes one at a time, in revision order, across files', async () => {
      let resolveButton: (v: string) => void = () => {}
      ;(napi.extractFileIncremental as any)
        .mockReturnValueOnce(new Promise<string>((r) => (resolveButton = r)))
        .mockResolvedValueOnce(update(3, ['Input']))

      const ctx = clientEnv()
      const buttonCall = (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx'))
      const inputCall = (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Input.tsx'))
      await Promise.resolve()
      await Promise.resolve()
      expect((napi.extractFileIncremental as any).mock.calls.map(([file]: [string]) => file)).toEqual([
        '/project/src/Button.tsx',
      ])

      resolveButton(update(2, ['Button']))
      await Promise.all([buttonCall, inputCall])

      expect(sentUpdates(ctx).map((u: any) => [u.revision, u.updatedKeys[0]])).toEqual([
        [2, 'Button'],
        [3, 'Input'],
      ])
    })

    // The session rebuilds itself for a resolution config, and the update is an ordinary delta.
    it('applies a tsconfig change like a source change and ignores other non-source files', async () => {
      const ctx = clientEnv()
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/tsconfig.json'))
      await (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/README.md'))
      expect((napi.extractFileIncremental as any).mock.calls.map(([file]: [string]) => file)).toEqual([
        '/project/tsconfig.json',
      ])
      expect(sentUpdates(ctx).map((u: any) => u.updatedKeys)).toEqual([['Button']])
    })

    it('keeps applying changes after one fails', async () => {
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
      ;(napi.extractFileIncremental as any)
        .mockRejectedValueOnce(new Error('boom'))
        .mockResolvedValueOnce(update(2, ['Input']))

      const ctx = clientEnv()
      await Promise.all([
        (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Button.tsx')),
        (plugin.hotUpdate as Function).call(ctx, makeOpts('/project/src/Input.tsx')),
      ])

      expect(sentUpdates(ctx).map((u: any) => u.updatedKeys[0])).toEqual(['Input'])
      consoleError.mockRestore()
    })
  })

  // ── injectDocgenInfo ───────────────────────────────────────────────────────

  describe('injectDocgenInfo', () => {
    const doc = (displayName: string) => ({
      displayName,
      description: '',
      props: {},
      methods: [],
      tags: {},
      composes: [],
    })
    const assignment = (displayName: string) =>
      `try { ${displayName}.__docgenInfo = ${JSON.stringify(doc(displayName))} } catch {}`
    const transform = (code: string, id: string) => (plugin.transform as any).handler.call(pluginContext(), code, id)

    beforeEach(async () => {
      ;(napi.sessionDocgenInfo as any).mockImplementation((_id: number, keys: string[]) =>
        JSON.stringify(Object.fromEntries(keys.map((key) => [key, doc(key)]))),
      )
      plugin = caliperDocgen({ srcDirs: ['src'], injectDocgenInfo: true })
      ;(plugin.configResolved as Function)({ root: '/project', command: 'build' })
      await (plugin.buildStart as Function).call(pluginContext())
    })

    it('appends a guarded __docgenInfo assignment for each component the module declares', async () => {
      expect(await transform('export function Button() {}', '/project/src/Button.tsx?v=1')).toEqual({
        code: `export function Button() {}\n${assignment('Button')}\n`,
        map: null,
      })
      expect(napi.sessionDocgenInfo).toHaveBeenCalledWith(42, ['Button'])
    })

    it('leaves a module without components, outside srcDirs, or with the option off untouched', async () => {
      expect(await transform('export const x = 1', '/project/src/util.ts')).toBeUndefined()
      expect(await transform('export function Button() {}', '/elsewhere/Button.tsx')).toBeUndefined()
      const off = caliperDocgen({ srcDirs: ['src'] })
      ;(off.configResolved as Function)({ root: '/project', command: 'build' })
      await expect(
        (off.transform as any).handler.call(pluginContext(), 'x', '/project/src/Button.tsx'),
      ).resolves.toBeUndefined()
    })

    it('follows the session through changes: a new component gains docs, a removed one loses them', async () => {
      ;(napi.extractFileIncremental as any).mockResolvedValueOnce(update(2, ['Card'], ['Button']))
      await (plugin.watchChange as Function).call(pluginContext(), '/project/src/Card.tsx')
      expect(await transform('export function Button() {}', '/project/src/Button.tsx')).toBeUndefined()
      expect((await transform('export function Card() {}', '/project/src/Card.tsx')).code).toBe(
        `export function Card() {}\n${assignment('Card')}\n`,
      )
    })

    it('re-transforms the module of a component whose props changed through another file', async () => {
      ;(napi.extractFileIncremental as any).mockResolvedValueOnce(update(2, ['Button']))
      const buttonModule = { id: '/project/src/Button.tsx' }
      const typesModule = { id: '/project/src/types.ts' }
      const moduleGraph = {
        getModuleById: vi.fn(),
        getModulesByFile: vi.fn((file: string) => (file === buttonModule.id ? new Set([buttonModule]) : undefined)),
        invalidateModule: vi.fn(),
      }
      const ctx = { environment: { name: 'client', hot: { send: vi.fn() }, moduleGraph } }
      const opts = {
        file: typesModule.id,
        type: 'update',
        modules: [typesModule],
        timestamp: 1,
        read: vi.fn(),
        server: {},
      }
      expect(await (plugin.hotUpdate as Function).call(ctx, opts)).toEqual([typesModule, buttonModule])
      expect(moduleGraph.invalidateModule).toHaveBeenCalledWith(buttonModule)
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

  // ── `vite build --watch` ─────────────────────────────────────────────────
  //
  // Per Rollup's plugin hook contract, buildEnd fires once per rebuild in watch mode; closeWatcher fires once when
  // the watch process shuts down. hotUpdate never fires; Rollup reports changes through watchChange.

  describe('vite build --watch', () => {
    const watchBuild = () => {
      const p = caliperDocgen({ srcDirs: ['src', 'lib'] })
      ;(p.configResolved as Function)({ root: '/project', command: 'build', build: { watch: {} } })
      return p
    }

    it('does not close the session on buildEnd while watching', () => {
      const p = watchBuild()
      ;(p.buildEnd as Function)()
      ;(p.buildEnd as Function)() // a second rebuild's buildEnd
      expect(napi.closeSession).not.toHaveBeenCalled()
    })

    it('closes the session via closeWatcher when the watch process shuts down', () => {
      ;(watchBuild().closeWatcher as Function)()
      expect(napi.closeSession).toHaveBeenCalledWith(42)
    })

    it('watches every source directory and resolution config, so a change no bundle module imports still rebuilds', async () => {
      const ctx = pluginContext()
      await (watchBuild().buildStart as Function).call(ctx)
      expect(ctx.addWatchFile.mock.calls).toEqual([['/project/src'], ['/project/lib'], ['/project/tsconfig.json']])
    })

    it('applies a changed source file before the next rebuild loads the virtual module', async () => {
      const p = watchBuild()
      await (p.buildStart as Function).call(pluginContext())

      await (p.watchChange as Function)('/project/src/Button.tsx', { event: 'update' })
      await (p.watchChange as Function)('/project/src/notes.md', { event: 'update' })
      await (p.buildStart as Function).call(pluginContext())

      expect((napi.extractFileIncremental as any).mock.calls.map(([file]: [string]) => file)).toEqual([
        '/project/src/Button.tsx',
      ])
      expect(napi.initializeSession).toHaveBeenCalledTimes(1)
      const code = await (p.load as Function).call(pluginContext(), '\0virtual:caliper-docgen')
      expect(code).toBe(`export default ${SNAPSHOT}`)
    })
  })

  // ── buildStart ─────────────────────────────────────────────────────────────

  describe('buildStart', () => {
    it('runs cold extraction when the command is "build"', async () => {
      ;(plugin.configResolved as Function)({ root: '/project', command: 'build' })
      await (plugin.buildStart as Function).call(pluginContext())
      expect(napi.initializeSession).toHaveBeenCalledWith(42, expect.objectContaining({ srcDirs: ['/project/src'] }))
    })

    it('does not run cold extraction when the command is "serve" (configureServer handles it)', async () => {
      ;(plugin.configResolved as Function)({ root: '/project', command: 'serve' })
      await (plugin.buildStart as Function).call(pluginContext())
      expect(napi.initializeSession).not.toHaveBeenCalled()
    })

    it('fails the build when extraction fails, instead of shipping an empty virtual module', async () => {
      ;(napi.initializeSession as any).mockRejectedValueOnce(new Error('no source directories'))
      ;(plugin.configResolved as Function)({ root: '/project', command: 'build' })
      const ctx = pluginContext()
      await expect((plugin.buildStart as Function).call(ctx)).rejects.toThrow(
        '[caliper-docgen] extraction failed: Error: no source directories',
      )
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

  // ── loadDependencies ───────────────────────────────────────────────────────

  describe('loadDependencies', () => {
    it('forwards loadDependencies to the NAPI call', () => {
      const p = caliperDocgen({ srcDirs: ['src'], loadDependencies: true })
      ;(p.configResolved as Function)({ root: '/project' })
      expect(napi.createSession).toHaveBeenCalledWith(expect.objectContaining({ loadDependencies: true }))
    })

    it('forwards collapseInheritedProps to the NAPI call', () => {
      const p = caliperDocgen({ srcDirs: ['src'], loadDependencies: true, collapseInheritedProps: 'none' })
      ;(p.configResolved as Function)({ root: '/project' })
      expect(napi.createSession).toHaveBeenCalledWith(
        expect.objectContaining({ loadDependencies: true, collapseInheritedProps: 'none' }),
      )
    })

    it('leaves loadDependencies unset when not passed', () => {
      const p = caliperDocgen({ srcDirs: ['src'] })
      ;(p.configResolved as Function)({ root: '/project' })
      expect(napi.createSession).toHaveBeenCalledWith(expect.objectContaining({ loadDependencies: undefined }))
    })
  })

  // ── exclude pass-through ───────────────────────────────────────────────────

  describe('exclude options forwarding', () => {
    it('forwards exclude to the NAPI call', () => {
      const p = caliperDocgen({ srcDirs: ['src'], exclude: ['**/*.stories.tsx'] })
      ;(p.configResolved as Function)({ root: '/project' })
      expect(napi.createSession).toHaveBeenCalledWith(expect.objectContaining({ exclude: ['**/*.stories.tsx'] }))
    })

    it('leaves exclude unset when not passed', () => {
      const p = caliperDocgen({ srcDirs: ['src'] })
      ;(p.configResolved as Function)({ root: '/project' })
      expect(napi.createSession).toHaveBeenCalledWith(expect.objectContaining({ exclude: undefined }))
    })
  })

  // ── Windows paths ──────────────────────────────────────────────────────────

  describe('Windows paths', () => {
    it('resolves a drive-letter srcDir and tracks backslash file paths under it', async () => {
      const p = caliperDocgen({ srcDirs: ['C:\\project\\src', 'lib'] })
      ;(p.configResolved as Function)({ root: 'C:\\project' })
      expect(napi.createSession).toHaveBeenLastCalledWith(
        expect.objectContaining({ srcDirs: ['C:/project/src', 'C:/project/lib'] }),
      )
      const ctx = { environment: { name: 'client', hot: { send: vi.fn() } } }
      const opts = (file: string) => ({
        file,
        modules: [],
        type: 'update' as const,
        timestamp: 1,
        read: vi.fn(),
        server: {} as any,
      })
      await (p.hotUpdate as Function).call(ctx, opts('C:\\project\\src\\Button.tsx'))
      expect(napi.extractFileIncremental).toHaveBeenCalledTimes(1)
      await (p.hotUpdate as Function).call(ctx, opts('C:\\project\\other\\Card.tsx'))
      expect(napi.extractFileIncremental).toHaveBeenCalledTimes(1)
    })
  })

  // ── htmlAttributes ─────────────────────────────────────────────────────────

  describe('htmlAttributes', () => {
    it('forwards htmlAttributes, ahead of skipHtmlProps', () => {
      const p = caliperDocgen({ srcDirs: ['src'], htmlAttributes: 'full', skipHtmlProps: true })
      ;(p.configResolved as Function)({ root: '/project' })
      expect(napi.createSession).toHaveBeenLastCalledWith(expect.objectContaining({ htmlAttributes: 'full' }))
      const q = caliperDocgen({ srcDirs: ['src'], skipHtmlProps: true })
      ;(q.configResolved as Function)({ root: '/project' })
      expect(napi.createSession).toHaveBeenLastCalledWith(expect.objectContaining({ htmlAttributes: 'none' }))
    })
  })

  // ── diagnostics ────────────────────────────────────────────────────────────

  describe('diagnostics', () => {
    const diagnostic = (severity: 'error' | 'warning' | 'info', message: string) => ({
      severity,
      message,
      file: '/project/src/Button.tsx',
      line: 3,
      column: 5,
      help: null,
      code: 'x',
    })
    const output = JSON.stringify({
      schemaVersion: 2,
      components: {},
      enums: {},
      stats: {},
      diagnostics: [diagnostic('error', 'bad'), diagnostic('warning', 'iffy'), diagnostic('info', 'fyi')],
    })
    const logger = () => ({ error: vi.fn(), warn: vi.fn(), info: vi.fn() })

    async function build(options: Partial<Parameters<typeof caliperDocgen>[0]>, log = logger()) {
      ;(napi.initializeSession as any).mockResolvedValue(output)
      const p = caliperDocgen({ srcDirs: ['src'], ...options })
      ;(p.configResolved as Function)({ root: '/project', command: 'build', logger: log })
      await (p.buildStart as Function).call(pluginContext())
      return log
    }

    it('logs errors and warnings once, with their location, and not info, by default', async () => {
      const log = await build({})
      expect(log.error).toHaveBeenCalledExactlyOnceWith('[caliper-docgen] /project/src/Button.tsx:3:5 bad')
      expect(log.warn).toHaveBeenCalledExactlyOnceWith('[caliper-docgen] /project/src/Button.tsx:3:5 iffy')
      expect(log.info).not.toHaveBeenCalled()
    })

    it('honors logLevel and calls onDiagnostic for every diagnostic', async () => {
      const seen: string[] = []
      const log = await build({ logLevel: 'error', onDiagnostic: (d) => seen.push(d.message) })
      expect(log.error).toHaveBeenCalledTimes(1)
      expect(log.warn).not.toHaveBeenCalled()
      expect(seen).toEqual(['bad', 'iffy', 'fyi'])
      const silent = await build({ logLevel: 'silent' })
      expect(silent.error).not.toHaveBeenCalled()
    })

    it('reports a diagnostic an update adds once, and again after an update removes it', async () => {
      const log = await build({})
      const added = diagnostic('warning', 'new')
      const change = (revision: number, diagnostics: unknown[], removedDiagnostics: unknown[] = []) =>
        JSON.stringify({ ...JSON.parse(update(revision, [])), diagnostics, removedDiagnostics })
      const p = caliperDocgen({ srcDirs: ['src'] })
      ;(p.configResolved as Function)({ root: '/project', command: 'serve', logger: log })
      const ctx = { environment: { name: 'client', hot: { send: vi.fn() } } }
      const opts = {
        file: '/project/src/Button.tsx',
        modules: [],
        type: 'update' as const,
        timestamp: 1,
        read: vi.fn(),
        server: {} as any,
      }
      const warnsBefore = log.warn.mock.calls.length
      ;(napi.extractFileIncremental as any)
        .mockResolvedValueOnce(change(2, [added]))
        .mockResolvedValueOnce(change(3, [added]))
        .mockResolvedValueOnce(change(4, [], [added]))
        .mockResolvedValueOnce(change(5, [added]))
      for (let i = 0; i < 4; i++) await (p.hotUpdate as Function).call(ctx, opts)
      expect(log.warn.mock.calls.length - warnsBefore).toBe(2)
    })
  })
})
