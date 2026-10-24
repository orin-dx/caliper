import { readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import type * as Napi from 'caliper-docgen-napi'
import type { Diagnostic, IncrementalUpdate, RdtComponentDoc } from 'caliper-docgen-napi'
import { localNameOf } from './exports.js'

/** The binding calls the session makes, so tests can stand in for the native module. */
export type SessionBinding = Pick<
  typeof Napi,
  'createSession' | 'initializeSessionIndex' | 'ensureFresh' | 'sessionDocgenInfo' | 'closeSession'
>

export interface SessionOptions {
  /** Absolute source directories. */
  srcDirs: string[]
  exclude?: string[]
  tsconfigPath?: string
  skipHtmlProps?: boolean
  htmlAttributes?: Napi.JsExtractOptions['htmlAttributes']
  loadDependencies?: boolean
  collapseInheritedProps?: Napi.JsExtractOptions['collapseInheritedProps']
}

interface ComponentRef {
  filePath: string
  displayName: string
}

/** What `keyOf` and `degradedBy` read, derived from the components and diagnostics. */
interface Lookups {
  byFile: Map<string, string[]>
  byName: Map<string, string[]>
  degradedBy: Map<string, Set<string>>
}

function push(map: Map<string, string[]>, at: string, key: string): void {
  const keys = map.get(at)
  if (keys) keys.push(key)
  else map.set(at, [key])
}

function diagnosticId(diagnostic: Diagnostic): string {
  return JSON.stringify(diagnostic)
}

/**
 * One watch session over the source directories, created on first use. Storybook gives no file events to a docgen
 * worker, so `fresh()` has the native session compare every source file's mtime and size with the last look and apply
 * the changed ones before a request is answered: an answer never trails a saved edit. The session holds the props;
 * this class holds only where each component lives and which ones a diagnostic says are incomplete.
 */
export class DocgenSession {
  private readonly components = new Map<string, ComponentRef>()
  private readonly diagnostics = new Map<string, Diagnostic>()
  /** Rebuilt from `components` and `diagnostics` on the first lookup after either changes. */
  private index: Lookups | undefined
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
      htmlAttributes: this.options.htmlAttributes ?? (this.options.skipHtmlProps ? 'none' : undefined),
      loadDependencies: this.options.loadDependencies,
      collapseInheritedProps: this.options.collapseInheritedProps,
    }
  }

  private async initialize(): Promise<void> {
    this.sessionId = this.napi.createSession(this.napiOptions())
    const output = JSON.parse(await this.napi.initializeSessionIndex(this.sessionId, this.napiOptions())) as {
      components: Record<string, ComponentRef>
      diagnostics: Diagnostic[]
    }
    for (const [key, entry] of Object.entries(output.components)) {
      this.components.set(key, { filePath: resolve(entry.filePath), displayName: entry.displayName })
    }
    for (const diagnostic of output.diagnostics) this.diagnostics.set(diagnosticId(diagnostic), diagnostic)
    this.index = undefined
  }

  private apply(update: IncrementalUpdate): void {
    if (update.revision <= this.latestRevision) return
    this.latestRevision = update.revision
    this.index = undefined
    for (const key of update.removedKeys) this.components.delete(key)
    update.updatedKeys.forEach((key, i) => {
      const entry = update.updatedComponents[i]
      if (entry) this.components.set(key, { filePath: resolve(entry.filePath), displayName: entry.displayName })
    })
    for (const diagnostic of update.removedDiagnostics) this.diagnostics.delete(diagnosticId(diagnostic))
    for (const diagnostic of update.diagnostics) this.diagnostics.set(diagnosticId(diagnostic), diagnostic)
  }

  private async check(): Promise<void> {
    this.apply(JSON.parse(await this.napi.ensureFresh(this.sessionId!)) as IncrementalUpdate)
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

  private lookups(): Lookups {
    if (this.index) return this.index
    const byFile = new Map<string, string[]>()
    const byName = new Map<string, string[]>()
    for (const [key, ref] of this.components) {
      push(byFile, ref.filePath, key)
      push(byName, ref.displayName, key)
    }
    const degradedBy = new Map<string, Set<string>>()
    for (const diagnostic of this.diagnostics.values()) {
      if (diagnostic.severity === 'info') continue
      for (const key of diagnostic.affects ?? []) {
        const messages = degradedBy.get(key) ?? new Set<string>()
        messages.add(diagnostic.message)
        degradedBy.set(key, messages)
      }
    }
    return (this.index = { byFile, byName, degradedBy })
  }

  /**
   * The key of the component `exportName` names in `path`. A file's only component answers for its default export;
   * a path with no components (a barrel that re-exports them) falls back to the one component with that name.
   */
  keyOf(path: string, exportName: string): string | undefined {
    const { byFile, byName } = this.lookups()
    const inFile = byFile.get(resolve(path))
    if (inFile) {
      const named = inFile.find((key) => this.components.get(key)?.displayName === exportName)
      if (named) return named
      return inFile.length === 1 && exportName === 'default' ? inFile[0] : undefined
    }
    if (exportName === 'default') return undefined
    const sameName = byName.get(exportName)
    return sameName?.length === 1 ? sameName[0] : undefined
  }

  /**
   * `keyOf`, and when the file holds several components and none has the export's name, the component the module's
   * source exports under it (`export default Card`, `export { Inner as Card }`), then, for a default import, the one
   * named as the story named its import (`importedAs`).
   */
  async resolveExport(path: string, exportName: string, importedAs?: string): Promise<string | undefined> {
    const key = this.keyOf(path, exportName)
    if (key) return key
    const file = resolve(path)
    const inFile = this.lookups().byFile.get(file)
    if (!inFile) return undefined
    const named = (name: string | undefined) => inFile.find((each) => this.components.get(each)?.displayName === name)
    const source = await readFile(file, 'utf8').catch(() => '')
    return named(localNameOf(source, exportName)) ?? (exportName === 'default' ? named(importedAs) : undefined)
  }

  /**
   * The key of the sub-component `property` of `namespace` (`Select.Option`): the component with that display name,
   * the one beside `near` if several share it.
   */
  keyOfMember(namespaces: string[], property: string, near: string): string | undefined {
    const { byName } = this.lookups()
    const matches = [...new Set(namespaces.flatMap((namespace) => byName.get(`${namespace}.${property}`) ?? []))]
    if (matches.length <= 1) return matches[0]
    const dir = dirname(resolve(near))
    const beside = matches.filter((key) => dirname(this.components.get(key)?.filePath ?? '') === dir)
    return beside.length === 1 ? beside[0] : undefined
  }

  /** The display names of the components in `path`, to say what a failed lookup had to choose from. */
  namesIn(path: string): string[] {
    const keys = this.lookups().byFile.get(resolve(path)) ?? []
    return keys.flatMap((key) => this.components.get(key)?.displayName ?? [])
  }

  /** Whether a warning or error says `key`'s props may be incomplete. */
  degraded(key: string): boolean {
    return this.degradedBy(key).length > 0
  }

  /** The messages of the warnings and errors that say `key`'s props may be incomplete. */
  degradedBy(key: string): string[] {
    return [...(this.lookups().degradedBy.get(key) ?? [])]
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
