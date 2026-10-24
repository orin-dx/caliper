import { readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import type * as Napi from 'caliper-docgen-napi'
import type { Diagnostic, IncrementalUpdate, RdtComponentDoc } from 'caliper-docgen-napi'
import { localNameOf } from './exports.js'

/** The binding calls the session makes, so tests can stand in for the native module. */
export type SessionBinding = Pick<
  typeof Napi,
  | 'createSession'
  | 'initializeSessionIndex'
  | 'ensureFresh'
  | 'sessionDocgenInfo'
  | 'sessionComponentAnswers'
  | 'sessionWarmUp'
  | 'closeSession'
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
  /**
   * Resolve a component when a story asks for it rather than every component at start. Default: `true`. The answers
   * are the same; a session that serves a few components of many starts and holds less.
   */
  lazy?: boolean
  /**
   * After a lazy session starts, resolve a few components in the background so the dependency declarations and memos
   * the stories share are loaded before the first one asks. Default: `true`.
   */
  warmUp?: boolean
}

/** How many components the background warm-up resolves, spread over the project. */
const WARM_UP_COMPONENTS = 8

/** What the session says about one component: its documentation and why its props may be incomplete. */
export interface Documentation {
  doc: RdtComponentDoc | undefined
  /** The messages of the warnings and errors that say the props may be incomplete; empty when none does. */
  degradedBy: string[]
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

/** The distinct messages of the warnings and errors among `diagnostics`; info says nothing about the props. */
function incompleteMessages(diagnostics: Diagnostic[]): string[] {
  return [...new Set(diagnostics.filter((diagnostic) => diagnostic.severity !== 'info').map((d) => d.message))]
}

function diagnosticId(diagnostic: Diagnostic): string {
  return JSON.stringify(diagnostic)
}

/**
 * One watch session over the source directories, created on first use. Storybook gives no file events to a docgen
 * worker, so `fresh()` has the native session compare every source file's mtime and size with the last look and apply
 * the changed ones before a request is answered: an answer never trails a saved edit. The session holds the props;
 * this class holds only where each component lives and, for an eager session, which ones a diagnostic says are
 * incomplete. A lazy session (the default) resolves a component when `documentation` asks, and holds no answers.
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
      lazy: this.lazy,
    }
  }

  private get lazy(): boolean {
    return this.options.lazy ?? true
  }

  private async initialize(): Promise<void> {
    this.sessionId = this.napi.createSession(this.napiOptions())
    await this.reindex()
    if (this.lazy && this.options.warmUp !== false) {
      // Not awaited: the first story answers as soon as it can, and shares what the warm-up has loaded by then.
      this.napi.sessionWarmUp(this.sessionId, WARM_UP_COMPONENTS).catch(() => {})
    }
  }

  /** Reads where every component is declared (and, for an eager session, which diagnostics affect them). */
  private async reindex(): Promise<void> {
    const output = JSON.parse(await this.napi.initializeSessionIndex(this.sessionId!, this.napiOptions())) as {
      components: Record<string, ComponentRef>
      diagnostics: Diagnostic[]
    }
    this.components.clear()
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
    const update = JSON.parse(await this.napi.ensureFresh(this.sessionId!)) as IncrementalUpdate
    if (this.lazy) {
      // A lazy update names the keys an edit can change but carries no component to place them, so look again at where
      // the components are; resolving none of them, that costs a parse-level listing.
      if (update.revision > this.latestRevision) {
        this.latestRevision = update.revision
        if (update.updatedKeys.length > 0 || update.removedKeys.length > 0) await this.reindex()
      }
      return
    }
    this.apply(update)
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

  /** Whether a warning or error says `key`'s props may be incomplete. An eager session only: see `documentation`. */
  degraded(key: string): boolean {
    return this.degradedBy(key).length > 0
  }

  /** The messages of the warnings and errors that say `key`'s props may be incomplete. */
  degradedBy(key: string): string[] {
    return [...(this.lookups().degradedBy.get(key) ?? [])]
  }

  /**
   * The documentation of `key` and why its props may be incomplete. A lazy session resolves the component now, or
   * answers from what an earlier story resolved; nothing is kept here, so an edit cannot leave a stale answer behind.
   */
  async documentation(key: string): Promise<Documentation> {
    if (!this.lazy) return { doc: this.docgenInfo(key), degradedBy: this.degradedBy(key) }
    if (this.sessionId === undefined) return { doc: undefined, degradedBy: [] }
    const answers = JSON.parse(this.napi.sessionComponentAnswers(this.sessionId, [key])) as Record<
      string,
      { docgenInfo: RdtComponentDoc; diagnostics: Diagnostic[] }
    >
    const answer = answers[key]
    return { doc: answer?.docgenInfo, degradedBy: answer ? incompleteMessages(answer.diagnostics) : [] }
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
