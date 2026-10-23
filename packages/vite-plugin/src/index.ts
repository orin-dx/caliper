import type { Plugin, ResolvedConfig, ViteDevServer, HotUpdateOptions, EnvironmentModuleNode } from 'vite'
import * as napi from 'caliper-docgen-napi'
import type { ExtractionOutput, IncrementalUpdate } from 'caliper-docgen-napi'

const VIRTUAL_ID = 'virtual:caliper-docgen'
const RESOLVED_VIRTUAL_ID = `\0${VIRTUAL_ID}`

/** A session's cold extraction is revision 1; every change that alters the output publishes the next one. */
const FIRST_REVISION = 1

export interface CaliperDocgenOptions {
  srcDirs: string[]
  exclude?: string[]
  skipHtmlProps?: boolean
  /** Load the type declarations your props reference from installed packages. Off by default. */
  loadDependencies?: boolean
  /**
   * Which inherited sources of more than 30 props to list as one inherited layer instead of prop by prop: those from
   * loaded dependencies (the default), every inherited source, or none.
   */
  collapseInheritedProps?: 'dependencies' | 'all' | 'none'
}

/**
 * What `caliper-docgen:update` carries to the client. Events arrive in `revision` order; each is a delta from the one
 * before. Delete `removedKeys`, set `updatedKeys[i]` to `updatedComponents[i]`, merge `props` into the output's
 * `props`, drop `removedDiagnostics` and add `diagnostics`, and the client holds the same output the virtual module
 * serves, less any props no component refers to any more.
 */
export interface CaliperDocgenUpdate {
  file: string
  revision: number
  updatedKeys: string[]
  updatedComponents: IncrementalUpdate['updatedComponents']
  /** Every prop `updatedComponents` refer to, by id. */
  props: IncrementalUpdate['props']
  removedKeys: string[]
  /** Diagnostics this revision added. */
  diagnostics: IncrementalUpdate['diagnostics']
  /** Diagnostics this revision no longer has. */
  removedDiagnostics: IncrementalUpdate['removedDiagnostics']
}

export function caliperDocgen(options: CaliperDocgenOptions): Plugin {
  let sessionId: number
  let root: string
  let command: 'build' | 'serve' | undefined
  // `vite build --watch`: Rollup's buildEnd hook fires once per rebuild, not
  // once when the watch process actually shuts down — closeWatcher is the
  // correct once-per-process hook for that. See buildEnd's own comment below.
  let isWatchBuild = false
  let resolvedSrcDirs: string[] = []
  // Holds the in-flight init promise so hotUpdate and load can await it.
  let initPromise: Promise<void> | null = null
  // The newest revision this plugin has seen; an update that reports it again changed nothing.
  let latestRevision = FIRST_REVISION
  // Changes are applied and announced one at a time: each update is a delta from the revision before it, so a
  // client must receive them in revision order. The session serializes writers anyway.
  let previousChange: Promise<unknown> = Promise.resolve()
  // `vite build --watch` changes applied before the next rebuild's load().
  let pendingWatchChanges: Promise<unknown>[] = []
  let extracted = false

  function napiOptions() {
    return {
      srcDirs: resolvedSrcDirs,
      exclude: options.exclude,
      htmlAttributes: options.skipHtmlProps ? ('none' as const) : undefined,
      loadDependencies: options.loadDependencies,
      collapseInheritedProps: options.collapseInheritedProps,
    }
  }

  function isSrcFile(file: string): boolean {
    return (file.endsWith('.tsx') || file.endsWith('.ts')) && resolvedSrcDirs.some((dir) => file.startsWith(`${dir}/`))
  }

  /** A source file, or a tsconfig, `package.json` or lockfile whose change rebuilds the session. */
  function isTrackedFile(file: string): boolean {
    return isSrcFile(file) || napi.isResolutionConfig(sessionId, file)
  }

  /** Runs `task` after every earlier one has finished, failed or not. */
  function inOrder<T>(task: () => Promise<T>): Promise<T> {
    const earlier = previousChange
    const run = (async () => {
      try {
        await earlier
      } catch {
        // An earlier change's failure belongs to its own caller.
      }
      return task()
    })()
    previousChange = run
    return run
  }

  /** Applies one file change to the session; `undefined` when it produced no new revision. */
  async function applyChange(file: string): Promise<IncrementalUpdate | undefined> {
    const update = JSON.parse(await napi.extractFileIncremental(file, sessionId, napiOptions())) as IncrementalUpdate
    if (update.revision <= latestRevision) return undefined
    latestRevision = update.revision
    return update
  }

  return {
    name: 'caliper-docgen',

    configResolved(config: ResolvedConfig) {
      ;({ root, command } = config)
      isWatchBuild = command === 'build' && Boolean(config.build?.watch)
      // A trailing slash (relative `'src/'`, or an already-absolute dir
      // ending in `/`) must be stripped here — isSrcFile's `startsWith(`${dir}/`)`
      // check would otherwise require a double slash no real file path has,
      // silently excluding every file under that srcDir from HMR.
      resolvedSrcDirs = options.srcDirs.map((d) => {
        const abs = d.startsWith('/') ? d : `${root}/${d}`
        return abs.endsWith('/') ? abs.slice(0, -1) : abs
      })
      // sessionId is foundational to every other hook, so a failure here
      // can't be swallowed — log for discoverability, then rethrow so Vite's
      // own config-resolution fails loudly instead of every later hook
      // failing confusingly against an undefined sessionId.
      try {
        sessionId = napi.createSession(napiOptions())
      } catch (error) {
        console.error('[caliper-docgen] createSession failed:', error)
        throw error
      }
    },

    // A build has no dev server to run the cold extraction, and a build that
    // cannot extract must fail rather than ship an empty virtual module.
    async buildStart() {
      if (command !== 'build') return
      if (isWatchBuild) {
        // Source files no bundle module imports must still trigger a rebuild, since the docs come from them, and so must
        // the configuration that decides how their imports resolve.
        for (const dir of resolvedSrcDirs) this.addWatchFile(dir)
        for (const file of napi.resolutionConfigFiles(sessionId)) this.addWatchFile(file)
        await Promise.all(pendingWatchChanges)
        pendingWatchChanges = []
      }
      // A watch rebuild reuses the session; its changes arrived through watchChange.
      if (extracted) return
      try {
        await napi.initializeSession(sessionId, napiOptions())
        extracted = true
      } catch (error) {
        this.error(`[caliper-docgen] extraction failed: ${String(error)}`)
      }
    },

    // `vite build --watch` never calls hotUpdate; Rollup reports each changed file here before rebuilding.
    async watchChange(id: string) {
      if (!isTrackedFile(id)) return
      const change = inOrder(async () => {
        try {
          await applyChange(id)
        } catch (error) {
          console.error('[caliper-docgen] Failed to apply the change:', error)
        }
      })
      pendingWatchChanges.push(change)
      await change
    },

    configureServer(server: ViteDevServer) {
      // Vite guarantees configResolved fires before configureServer;
      // sessionId and resolvedSrcDirs are safe to use here.
      //
      // This hook must return its teardown function synchronously — it can't
      // be `async` or `await` the extraction here without breaking that
      // contract, so the fire-and-forget .then()/.catch() chain below is
      // intentional, not an oversight.
      /* oxlint-disable promise/prefer-await-to-then, promise/always-return, promise/prefer-await-to-callbacks */
      initPromise = napi
        .initializeSession(sessionId, napiOptions())
        .then((json) => {
          const output = JSON.parse(json) as ExtractionOutput
          // environments.client.hot is Vite 6+ API; cast avoids typing fight across patch versions
          ;(server as any).environments?.client?.hot?.send('caliper-docgen:ready', { components: output.components })
        })
        .catch((error: unknown) => {
          console.error('[caliper-docgen] initializeSession failed:', error)
        })
      /* oxlint-enable promise/prefer-await-to-then, promise/always-return, promise/prefer-await-to-callbacks */

      // A tsconfig, `package.json` or lockfile above the source directories changes how every import resolves; its
      // change reaches hotUpdate like a source file's and rebuilds the session there.
      server.watcher.add(napi.resolutionConfigFiles(sessionId))

      // buildEnd isn't called in dev mode — this is the only reliable cleanup hook.
      return () => {
        napi.closeSession(sessionId)
      }
    },

    async hotUpdate(opts: HotUpdateOptions) {
      // Vite 6+ supplies this.environment in the hotUpdate context.
      // We cast rather than annotating `this:` to avoid fighting the Plugin interface's
      // declared this-type across different Vite patch versions.
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const env = (this as any).environment as
        | {
            name: string
            hot: { send(event: string, data?: unknown): void }
            moduleGraph?: {
              getModuleById(id: string): EnvironmentModuleNode | undefined
              invalidateModule(mod: EnvironmentModuleNode): void
            }
          }
        | undefined

      if (!env || env.name !== 'client') return
      if (!isTrackedFile(opts.file)) return

      if (initPromise) await initPromise

      // Apply and announce inside one turn of the queue, so events leave in revision order.
      const update = await inOrder(async () => {
        let applied: IncrementalUpdate | undefined
        try {
          applied = await applyChange(opts.file)
        } catch (error) {
          console.error('[caliper-docgen] Failed to apply the change:', error)
          return
        }
        if (!applied) return
        const payload: CaliperDocgenUpdate = {
          file: opts.file,
          revision: applied.revision,
          updatedKeys: applied.updatedKeys,
          updatedComponents: applied.updatedComponents,
          props: applied.props,
          removedKeys: applied.removedKeys,
          diagnostics: applied.diagnostics,
          removedDiagnostics: applied.removedDiagnostics,
        }
        env.hot.send('caliper-docgen:update', payload)
        return applied
      })
      if (!update) return

      // The virtual module serves the whole output, so its importers must reload it. Keeping `opts.modules` keeps
      // React Fast Refresh for the changed file; returning [] would suppress it.
      const virtual = env.moduleGraph?.getModuleById(RESOLVED_VIRTUAL_ID)
      if (!virtual) return
      env.moduleGraph!.invalidateModule(virtual)
      return [...opts.modules, virtual]
    },

    resolveId(id: string) {
      if (id === VIRTUAL_ID) return RESOLVED_VIRTUAL_ID
    },

    // Served from the session, so it always carries every change applied so far: removals, enums and diagnostics
    // included.
    async load(id: string) {
      if (id !== RESOLVED_VIRTUAL_ID) return
      if (initPromise) await initPromise
      try {
        return `export default ${await napi.initializeSession(sessionId, napiOptions())}`
      } catch (error) {
        this.error(`[caliper-docgen] extraction failed: ${String(error)}`)
      }
    },

    buildEnd() {
      // In `vite build --watch`, this fires once per rebuild (Rollup's own
      // build-hook contract), not once at process shutdown. Closing here
      // would make every rebuild after the first auto-vivify a brand new
      // session on its next buildStart (the Rust binding creates one for
      // any unrecognized/closed session id) — silently losing all
      // incremental-cache benefit on every rebuild. closeWatcher below is
      // the correct once-per-process hook for the watch-build case.
      if (isWatchBuild) return
      napi.closeSession(sessionId)
    },

    closeWatcher() {
      napi.closeSession(sessionId)
    },
  }
}
