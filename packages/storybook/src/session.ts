import { readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import type * as Napi from 'caliper-docgen-napi'
import type { Diagnostic, IncrementalUpdate, RdtComponentDoc, SessionIndex } from 'caliper-docgen-napi'
import { declaresExport, localNameOf } from './exports.js'
import { Fill } from './fill.js'
import { buildLookups, diagnosticId, incompleteMessages, type ComponentRef, type Lookups } from './lookups.js'
import { assertSchemaVersion, expandDiagnostics } from './schema.js'

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
  /** Read and write the on-disk caches. Default: `true`. */
  cache?: boolean
  loadDependencies?: boolean
  collapseInheritedProps?: Napi.JsExtractOptions['collapseInheritedProps']
  /**
   * Resolve a component when a story asks for it rather than every component at start. Default: `true`. The answers
   * are the same; a session that serves a few components of many starts and holds less.
   */
  lazy?: boolean
  /**
   * After a lazy session starts, resolve a few components in the background so the dependency declarations and memos
   * the stories share are loaded before the first one asks. Default: `false`: measured against the same stories, it
   * did not make the first story faster, because the warm-up and the story compete for the same threads.
   */
  warmUp?: boolean
  /**
   * Once this many distinct components were asked for, resolve the rest in one background pass. Default: `0`, never:
   * it lowers the median story latency but raises peak memory by 170 to 420 MB.
   */
  fillAfter?: number
}

/** How many components the background warm-up resolves, spread over the project. */
const WARM_UP_COMPONENTS = 8
const FILL_AFTER = 0
/** The most components `sessionWarmUp` takes: all of them. */
const ALL_COMPONENTS = 2 ** 32 - 1

/** What the session says about one component: its documentation and why its props may be incomplete. */
export interface Documentation {
  doc: RdtComponentDoc | undefined
  /** The messages of the warnings and errors that say the props may be incomplete; empty when none does. */
  degradedBy: string[]
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
  private readonly fill: Fill
  private readonly options: SessionOptions
  private readonly napi: SessionBinding

  constructor(options: SessionOptions, napi: SessionBinding) {
    this.options = options
    this.napi = napi
    this.fill = new Fill(options.fillAfter ?? FILL_AFTER)
  }

  private napiOptions(): Napi.JsExtractOptions {
    return {
      srcDirs: this.options.srcDirs,
      exclude: this.options.exclude,
      tsconfigPath: this.options.tsconfigPath,
      htmlAttributes: this.options.htmlAttributes ?? (this.options.skipHtmlProps ? 'none' : undefined),
      cache: this.options.cache,
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
    // Not awaited: the first story answers as soon as it can, and shares what the warm-up has loaded by then.
    if (this.lazy && this.options.warmUp === true) void this.warmUp(this.sessionId, WARM_UP_COMPONENTS)
  }

  /** Best effort: a background resolution that fails costs the stories the time it would have saved. */
  private async warmUp(sessionId: number, count: number): Promise<void> {
    try {
      await this.napi.sessionWarmUp(sessionId, count)
    } catch {
      // The stories resolve what they need themselves.
    }
  }

  /** Reads where every component is declared (and, for an eager session, which diagnostics affect them). */
  private async reindex(): Promise<void> {
    const json = await this.napi.initializeSessionIndex(this.sessionId!, this.napiOptions())
    const output = assertSchemaVersion(JSON.parse(json) as SessionIndex, 'session index')
    this.components.clear()
    for (const [key, entry] of Object.entries(output.components)) {
      this.components.set(key, { filePath: resolve(entry.filePath), displayName: entry.displayName })
    }
    for (const diagnostic of expandDiagnostics(output.diagnostics, output.nameLists)) {
      this.diagnostics.set(diagnosticId(diagnostic), diagnostic)
    }
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
    for (const diagnostic of expandDiagnostics(update.removedDiagnostics, update.nameLists)) {
      this.diagnostics.delete(diagnosticId(diagnostic))
    }
    for (const diagnostic of expandDiagnostics(update.diagnostics, update.nameLists)) {
      this.diagnostics.set(diagnosticId(diagnostic), diagnostic)
    }
  }

  private async check(): Promise<void> {
    const json = await this.napi.ensureFresh(this.sessionId!)
    const update = assertSchemaVersion(JSON.parse(json) as IncrementalUpdate, 'update')
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
    return (this.index ??= buildLookups(this.components, this.diagnostics.values()))
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
   * `keyOf`, else what the module's source exports under the name (`export default Card`, `export { Inner as Card }`),
   * else for a default import the component named as the story named it (`importedAs`), else the file's only
   * component when its source declares that export (the display name may differ).
   */
  async resolveExport(path: string, exportName: string, importedAs?: string): Promise<string | undefined> {
    const key = this.keyOf(path, exportName)
    if (key) return key
    const inFile = this.lookups().byFile.get(resolve(path))
    if (!inFile) return undefined
    const named = (name: string | undefined) => inFile.find((each) => this.components.get(each)?.displayName === name)
    const source = await readFile(resolve(path), 'utf8').catch(() => '')
    const sole = inFile.length === 1 && declaresExport(source, exportName) ? inFile[0] : undefined
    return named(localNameOf(source, exportName)) ?? (exportName === 'default' ? named(importedAs) : sole)
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
  documentation(key: string): Documentation {
    if (!this.lazy) return { doc: this.docgenInfo(key), degradedBy: this.degradedBy(key) }
    if (this.sessionId === undefined) return { doc: undefined, degradedBy: [] }
    const answers = JSON.parse(this.napi.sessionComponentAnswers(this.sessionId, [key])) as Record<
      string,
      { docgenInfo: RdtComponentDoc; diagnostics: Diagnostic[] }
    >
    const answer = answers[key]
    if (!answer) {
      // The listing includes a re-export of a package component that has nothing to show; it is no component.
      if (this.components.delete(key)) this.index = undefined
      return { doc: undefined, degradedBy: [] }
    }
    if (this.fill.asked(key)) void this.warmUp(this.sessionId, ALL_COMPONENTS)
    return { doc: answer.docgenInfo, degradedBy: incompleteMessages(answer.diagnostics) }
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
