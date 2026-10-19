import type { Plugin, ResolvedConfig, ViteDevServer, HotUpdateOptions } from 'vite'
import * as napi from 'caliper-docgen-napi'
import type { ExtractionOutput, IncrementalUpdate } from 'caliper-docgen-napi'

const VIRTUAL_ID = 'virtual:caliper-docgen'
const RESOLVED_VIRTUAL_ID = `\0${VIRTUAL_ID}`

export interface OxcDocgenOptions {
  srcDirs: string[]
  exclude?: string[]
  reactVersion?: 'react18' | 'react19'
  skipHtmlProps?: boolean
}

export function caliperDocgen(options: OxcDocgenOptions): Plugin {
  let sessionId: number
  let root: string
  let command: 'build' | 'serve' | undefined
  // `vite build --watch`: Rollup's buildEnd hook fires once per rebuild, not
  // once when the watch process actually shuts down — closeWatcher is the
  // correct once-per-process hook for that. See buildEnd's own comment below.
  let isWatchBuild = false
  let resolvedSrcDirs: string[] = []
  let currentOutput: ExtractionOutput = {
    components: {},
    enums: {},
    diagnostics: [],
    stats: {
      componentsExtracted: 0,
      componentsSkipped: 0,
      filesParsed: 0,
      dtsFilesParsed: 0,
      dtsCacheHits: 0,
      durationMs: 0,
      tier1Count: 0,
      tier3Count: 0,
      opaqueCount: 0,
    },
  }
  // Holds the in-flight init promise so hotUpdate can await it before first incremental call.
  let initPromise: Promise<void> | null = null
  // Per-file monotonic sequence guard: two overlapping hotUpdate calls for
  // the same file (e.g. a rapid double-save) can have their async NAPI
  // calls resolve out of order. Without this, an older call finishing last
  // would silently overwrite currentOutput/the HMR payload with stale data
  // even though a newer call already applied fresher results.
  let hotUpdateSeq = 0
  const latestSeqByFile = new Map<string, number>()

  function napiOptions() {
    return {
      srcDirs: resolvedSrcDirs,
      exclude: options.exclude,
      reactVersion: options.reactVersion,
      htmlAttributes: options.skipHtmlProps ? ('none' as const) : undefined,
    }
  }

  // configureServer (dev-only) is where cold extraction normally runs and
  // populates `currentOutput` for the virtual module. Vite never calls
  // configureServer during `vite build`, so without this, a production
  // build's virtual module always resolved to the empty placeholder below.
  async function coldExtract(): Promise<boolean> {
    const json = await napi.initializeSession(sessionId, napiOptions())
    try {
      currentOutput = JSON.parse(json) as ExtractionOutput
      return true
    } catch (error) {
      console.error('[caliper-docgen] Failed to parse initializeSession output:', error)
      return false
    }
  }

  function isSrcFile(file: string): boolean {
    return (file.endsWith('.tsx') || file.endsWith('.ts')) && resolvedSrcDirs.some((dir) => file.startsWith(`${dir}/`))
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
      // can't be swallowed the way coldExtract's failures are — log for
      // discoverability, then rethrow so Vite's own config-resolution fails
      // loudly instead of every later hook failing confusingly against an
      // undefined sessionId.
      try {
        sessionId = napi.createSession(napiOptions())
      } catch (error) {
        console.error('[caliper-docgen] createSession failed:', error)
        throw error
      }
    },

    // Build-only counterpart to coldExtract() in configureServer; see that comment.
    async buildStart() {
      if (command !== 'build') return
      try {
        await coldExtract()
      } catch (error) {
        console.error('[caliper-docgen] initializeSession failed:', error)
      }
    },

    configureServer(server: ViteDevServer) {
      // Vite guarantees configResolved fires before configureServer;
      // sessionId and resolvedSrcDirs are safe to use here.
      //
      // This hook must return its teardown function synchronously — it can't
      // be `async` or `await` coldExtract() here without breaking that contract,
      // so the fire-and-forget .then()/.catch() chain below is intentional,
      // not an oversight.
      /* oxlint-disable promise/prefer-await-to-then, promise/always-return, promise/prefer-await-to-callbacks */
      initPromise = coldExtract()
        .then((ok) => {
          if (!ok) return
          // environments.client.hot is Vite 6+ API; cast avoids typing fight across patch versions
          ;(server as any).environments?.client?.hot?.send('caliper-docgen:ready', {
            components: currentOutput.components,
          })
        })
        .catch((error: unknown) => {
          console.error('[caliper-docgen] initializeSession failed:', error)
        })
      /* oxlint-enable promise/prefer-await-to-then, promise/always-return, promise/prefer-await-to-callbacks */

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
        | { name: string; hot: { send(event: string, data?: unknown): void } }
        | undefined

      if (!env || env.name !== 'client') return
      if (!isSrcFile(opts.file)) return

      if (initPromise) await initPromise

      const seq = ++hotUpdateSeq
      latestSeqByFile.set(opts.file, seq)

      const json = await napi.extractFileIncremental(opts.file, sessionId, napiOptions())

      // A newer hotUpdate call for this same file has already taken over —
      // its own result (in flight or already applied) is authoritative, so
      // applying this now-stale one would be a straight regression. Drop it
      // silently rather than race it, for both the success and the
      // malformed-JSON fallback path below.
      if (latestSeqByFile.get(opts.file) !== seq) return

      let update: IncrementalUpdate
      try {
        update = JSON.parse(json) as IncrementalUpdate
      } catch (error) {
        console.error('[caliper-docgen] Failed to parse extractFileIncremental output:', error)
        env.hot.send('caliper-docgen:update', { file: opts.file, updatedComponents: [], diagnostics: [] })
        return
      }

      for (const comp of update.updatedComponents) {
        currentOutput.components[comp.displayName] = comp
      }

      env.hot.send('caliper-docgen:update', {
        file: opts.file,
        updatedComponents: update.updatedComponents,
        diagnostics: update.diagnostics,
      })

      // Return undefined (NOT []) — returning [] suppresses React Fast Refresh
      // for the changed file. We want both: updated types AND component re-render.
    },

    resolveId(id: string) {
      if (id === VIRTUAL_ID) return RESOLVED_VIRTUAL_ID
    },

    load(id: string) {
      if (id !== RESOLVED_VIRTUAL_ID) return
      return `export default ${JSON.stringify(currentOutput)}`
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
