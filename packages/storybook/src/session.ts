import { dirname, resolve } from 'node:path'
import type * as Napi from 'caliper-docgen-napi'
import type { Diagnostic, IncrementalUpdate, RdtComponentDoc, SessionIndex } from 'caliper-docgen-napi'
import { toNapiOptions, type SessionOptions } from './options.js'
import {
  buildLookups,
  exportKey,
  incompleteMessages,
  only,
  type ImportName,
  refOf,
  type ComponentRef,
  type Lookups,
} from './lookups.js'
import { assertSchemaVersion } from './schema.js'

export type { SessionOptions } from './options.js'

/** The binding calls the session makes, so tests can stand in for the native module. */
export type SessionBinding = Pick<
  typeof Napi,
  | 'createSession'
  | 'initializeSessionIndex'
  | 'ensureFresh'
  | 'sessionComponentAnswers'
  | 'sessionPrefetch'
  | 'sessionCancel'
  | 'sessionCheckerSettled'
  | 'closeSession'
>

/**
 * What the session says about one component, as the binding decided it (`sessionComponentAnswers`): nothing here is
 * derived from the session index, for a lazy and an eager session alike.
 */
export interface Documentation {
  doc: RdtComponentDoc | undefined
  /** The messages of the answer's warnings and errors, which say the props may be incomplete; empty when none does. */
  degradedBy: string[]
  /**
   * The props are the structural ones because the checker had not answered within `semanticWaitMs`: once
   * `checkerSettled` says it has, asking again gets the checker's.
   */
  awaitsChecker: boolean
  /** The binding's revision of the files the answer was made at. */
  revision: number
}

/** One component's answer as the binding returns it. */
interface Answer {
  docgenInfo: RdtComponentDoc
  diagnostics: Diagnostic[]
  awaitsChecker: boolean
  revision: number
}

/**
 * One watch session over the source directories, created on first use. Storybook gives no file events to a docgen
 * worker, so `fresh()` has the native session compare every source file's mtime and size with the last look and apply
 * the changed ones before a request is answered: an answer never trails a saved edit. The session holds the props and
 * decides each answer; this class holds only where each component lives. A lazy session (the default) resolves a
 * component when `documentation` asks, and holds no answers.
 */
export class DocgenSession {
  private readonly components = new Map<string, ComponentRef>()
  /** Rebuilt from `components` on the first lookup after it changes. */
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

  /** The revision of the files the session last looked at; it rises with every look that found a change. */
  get revision(): number {
    return this.latestRevision
  }

  private napiOptions(): Napi.JsExtractOptions {
    return toNapiOptions(this.options, this.lazy)
  }

  private get lazy(): boolean {
    return this.options.lazy ?? true
  }

  private async initialize(): Promise<void> {
    this.sessionId = this.napi.createSession(this.napiOptions())
    await this.reindex()
  }

  /** Reads where every component is declared. */
  private async reindex(): Promise<void> {
    const json = await this.napi.initializeSessionIndex(this.sessionId!, this.napiOptions())
    const output = assertSchemaVersion(JSON.parse(json) as SessionIndex, 'session index')
    this.components.clear()
    for (const [key, entry] of Object.entries(output.components)) {
      this.components.set(key, refOf(entry))
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
      if (entry) this.components.set(key, refOf(entry))
    })
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
    const attempt = this.ready
    try {
      await attempt
    } catch (error) {
      // A failed start is not kept: the next request starts again, with nothing left over from this attempt. Of
      // several callers that shared it, the first to see it fail clears it.
      if (this.ready === attempt) {
        this.ready = undefined
        this.discard()
      }
      throw error
    }
    if (!this.checking) return this.look()
    this.queued ??= this.lookAfter(this.checking)
    return this.queued
  }

  /** Releases the native session a failed start created, if it got that far. */
  private discard(): void {
    if (this.sessionId === undefined) return
    try {
      this.napi.closeSession(this.sessionId)
    } catch {
      // The attempt already failed; the next one makes its own session.
    }
    this.sessionId = undefined
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
    return (this.index ??= buildLookups(this.components))
  }

  /**
   * The key of the component `path` exports as `exportName` (`default` for the default export), or, with `members`, of
   * the sub-component reached through them (`Menu.Item` beside `export default Menu` is `default`, `['Item']`).
   */
  keyOfExport(path: string, exportName: string, members: readonly string[] = []): string | undefined {
    const at = exportKey(resolve(path), exportName, members)
    return only(this.lookups().byExport.get(at))
  }

  /**
   * The key of the component `exportName` names in `path`. A path with no components is a barrel that re-exports them,
   * which `exports` does not record, so it falls back to the one component with that display name.
   */
  keyOf(path: string, exportName: string): string | undefined {
    const { byFile, byName } = this.lookups()
    if (byFile.has(resolve(path))) return this.keyOfExport(path, exportName)
    return exportName === 'default' ? undefined : only(byName.get(exportName))
  }

  /**
   * `keyOf`, else for a default export no component has, the component named as the story named its default import
   * (`importedAs`).
   */
  resolveExport(path: string, exportName: string, importedAs?: string): string | undefined {
    const key = this.keyOf(path, exportName)
    if (key || exportName !== 'default' || importedAs === undefined) return key
    const inFile = this.lookups().byFile.get(resolve(path))
    return only(inFile?.filter((each) => this.components.get(each)?.displayName === importedAs))
  }

  /** The key of `namespace.property` (`Select.Option`): attached to the export in `near`, else by display name beside it. */
  keyOfMember(namespace: ImportName, property: string, near: string): string | undefined {
    const attached = this.keyOfExport(near, namespace.exportName, [property])
    if (attached) return attached
    const { byName } = this.lookups()
    const names = [namespace.localName, namespace.exportName]
    const matches = [...new Set(names.flatMap((name) => byName.get(`${name}.${property}`) ?? []))]
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

  /**
   * The documentation of `key`, why its props may be incomplete and whether the checker is still answering, as the
   * binding decided them: one call for a lazy and an eager session. A lazy session resolves the component now, or
   * answers from what an earlier story resolved; nothing is kept here, so an edit cannot leave a stale answer behind.
   */
  documentation(key: string): Documentation {
    const none = { doc: undefined, degradedBy: [], awaitsChecker: false, revision: this.latestRevision }
    if (this.sessionId === undefined) return none
    const answers = JSON.parse(this.napi.sessionComponentAnswers(this.sessionId, [key])) as Record<string, Answer>
    const answer = answers[key]
    if (!answer) {
      // The listing includes a re-export of a package component that has nothing to show; it is no component.
      if (this.components.delete(key)) this.index = undefined
      return none
    }
    const { docgenInfo: doc, diagnostics, awaitsChecker, revision } = answer
    return { doc, degradedBy: incompleteMessages(diagnostics), awaitsChecker, revision }
  }

  /**
   * Resolves `keys` in the background, in order, the checker's answers included (kept), asked at the neighbour class:
   * after the component a story shows. Lazy only; waits for nobody.
   */
  async prefetch(keys: string[]): Promise<void> {
    if (!this.lazy || this.sessionId === undefined || keys.length === 0) return
    await this.napi.sessionPrefetch(this.sessionId, keys)
  }

  /** Drops the checker's queued requests for `keys` that nobody waits for: the client no longer shows them. */
  cancel(keys: string[]): void {
    if (!this.lazy || this.sessionId === undefined || keys.length === 0) return
    this.napi.sessionCancel(this.sessionId, keys)
  }

  /** Waits up to `waitMs` until the checker has answered `keys`: true when asking again gets its props (no edit since). */
  checkerSettled(keys: string[], waitMs: number): Promise<boolean> {
    if (this.sessionId === undefined) return Promise.resolve(false)
    return this.napi.sessionCheckerSettled(this.sessionId, keys, waitMs)
  }

  close(): void {
    if (this.sessionId !== undefined) this.napi.closeSession(this.sessionId)
  }
}
