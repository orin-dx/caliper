import { readdir, stat } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import type * as Napi from 'caliper-docgen-napi'
import type { Diagnostic, ExtractionOutput, IncrementalUpdate, RdtComponentDoc } from 'caliper-docgen-napi'

/** The binding calls the session makes, so tests can stand in for the native module. */
export type SessionBinding = Pick<
  typeof Napi,
  'createSession' | 'initializeSession' | 'extractFileIncremental' | 'sessionDocgenInfo' | 'closeSession'
>

export interface SessionOptions {
  /** Absolute source directories. */
  srcDirs: string[]
  exclude?: string[]
  tsconfigPath?: string
  skipHtmlProps?: boolean
}

interface ComponentRef {
  filePath: string
  displayName: string
}

const SOURCE_FILE = /\.(ts|tsx)$/

/** A file's identity for change detection: a save changes its mtime or its size. */
async function signature(path: string): Promise<string | undefined> {
  try {
    const stats = await stat(path)
    return `${stats.mtimeMs}:${stats.size}`
  } catch {
    return undefined
  }
}

/** Every `.ts`/`.tsx` file under `dir` and its signature, skipping `node_modules` and dot-directories. */
async function scan(dir: string, into: Map<string, string>): Promise<void> {
  let entries
  try {
    entries = await readdir(dir, { withFileTypes: true })
  } catch {
    return
  }
  await Promise.all(
    entries.map(async (entry) => {
      const path = join(dir, entry.name)
      if (entry.isDirectory()) {
        if (entry.name !== 'node_modules' && !entry.name.startsWith('.')) await scan(path, into)
      } else if (SOURCE_FILE.test(entry.name)) {
        const sig = await signature(path)
        if (sig) into.set(path, sig)
      }
    }),
  )
}

function diagnosticId(diagnostic: Diagnostic): string {
  return JSON.stringify(diagnostic)
}

/**
 * One watch session over the source directories, created on first use. Storybook gives no file events to a docgen
 * worker, so `fresh()` compares every source file's mtime and size with the last look and applies the changed ones
 * before a request is answered: an answer never trails a saved edit.
 */
export class DocgenSession {
  private readonly components = new Map<string, ComponentRef>()
  private readonly diagnostics = new Map<string, Diagnostic>()
  private files = new Map<string, string>()
  private sessionId: number | undefined
  private ready: Promise<void> | undefined
  private checking: Promise<void> | undefined
  private queued: Promise<void> | undefined
  private latestRevision = 0
  private readonly options: SessionOptions
  private readonly napi: SessionBinding

  constructor(options: SessionOptions, napi: SessionBinding) {
    this.options = options
    this.napi = napi
  }

  private napiOptions(): Napi.JsExtractOptions {
    return {
      srcDirs: this.options.srcDirs,
      exclude: this.options.exclude,
      tsconfigPath: this.options.tsconfigPath,
      htmlAttributes: this.options.skipHtmlProps ? 'none' : undefined,
    }
  }

  private async scanAll(): Promise<Map<string, string>> {
    const files = new Map<string, string>()
    await Promise.all(this.options.srcDirs.map((dir) => scan(dir, files)))
    return files
  }

  private async initialize(): Promise<void> {
    this.files = await this.scanAll()
    this.sessionId = this.napi.createSession(this.napiOptions())
    const output = JSON.parse(await this.napi.initializeSession(this.sessionId, this.napiOptions())) as ExtractionOutput
    for (const [key, entry] of Object.entries(output.components)) {
      this.components.set(key, { filePath: resolve(entry.filePath), displayName: entry.displayName })
    }
    for (const diagnostic of output.diagnostics) this.diagnostics.set(diagnosticId(diagnostic), diagnostic)
  }

  private apply(update: IncrementalUpdate): void {
    if (update.revision <= this.latestRevision) return
    this.latestRevision = update.revision
    for (const key of update.removedKeys) this.components.delete(key)
    update.updatedKeys.forEach((key, i) => {
      const entry = update.updatedComponents[i]
      if (entry) this.components.set(key, { filePath: resolve(entry.filePath), displayName: entry.displayName })
    })
    for (const diagnostic of update.removedDiagnostics) this.diagnostics.delete(diagnosticId(diagnostic))
    for (const diagnostic of update.diagnostics) this.diagnostics.set(diagnosticId(diagnostic), diagnostic)
  }

  private async check(): Promise<void> {
    const now = await this.scanAll()
    const changed = [...now].filter(([path, sig]) => this.files.get(path) !== sig).map(([path]) => path)
    const removed = [...this.files.keys()].filter((path) => !now.has(path))
    this.files = now
    for (const file of [...changed, ...removed]) {
      // One at a time: each update is a delta from the revision before it.
      // oxlint-disable-next-line no-await-in-loop
      const json = await this.napi.extractFileIncremental(file, this.sessionId!, this.napiOptions())
      this.apply(JSON.parse(json) as IncrementalUpdate)
    }
  }

  /**
   * Resolves once the session holds every edit saved before the call. Concurrent callers share one look at the
   * files; a caller arriving while a look is under way waits for it and then takes its own, since that look may
   * have started before the caller's edit.
   */
  async fresh(): Promise<void> {
    this.ready ??= this.initialize()
    await this.ready
    if (!this.checking) return this.look()
    this.queued ??= this.lookAfter(this.checking)
    return this.queued
  }

  private async lookAfter(earlier: Promise<void>): Promise<void> {
    try {
      await earlier
    } catch {
      // That look's failure belongs to its own callers.
    }
    this.queued = undefined
    return this.look()
  }

  /** Starts a look and publishes it in `checking` before yielding, so a caller arriving next sees it. */
  private async look(): Promise<void> {
    const look = this.check()
    this.checking = look
    try {
      await look
    } finally {
      if (this.checking === look) this.checking = undefined
    }
  }

  /**
   * The key of the component `exportName` names in `path`. A file's only component answers for its default export;
   * a path with no components (a barrel that re-exports them) falls back to the one component with that name.
   */
  keyOf(path: string, exportName: string): string | undefined {
    const file = resolve(path)
    const inFile = [...this.components].filter(([, ref]) => ref.filePath === file)
    if (inFile.length > 0) {
      const named = inFile.find(([, ref]) => ref.displayName === exportName)
      if (named) return named[0]
      return inFile.length === 1 && exportName === 'default' ? inFile[0]![0] : undefined
    }
    if (exportName === 'default') return undefined
    const byName = [...this.components].filter(([, ref]) => ref.displayName === exportName)
    return byName.length === 1 ? byName[0]![0] : undefined
  }

  /** Whether a warning or error says `key`'s props may be incomplete. */
  degraded(key: string): boolean {
    for (const diagnostic of this.diagnostics.values()) {
      if (diagnostic.severity !== 'info' && diagnostic.affects?.includes(key)) return true
    }
    return false
  }

  docgenInfo(key: string): RdtComponentDoc | undefined {
    if (this.sessionId === undefined) return undefined
    const docs = JSON.parse(this.napi.sessionDocgenInfo(this.sessionId, [key])) as Record<string, RdtComponentDoc>
    return docs[key]
  }

  close(): void {
    if (this.sessionId !== undefined) this.napi.closeSession(this.sessionId)
  }
}
