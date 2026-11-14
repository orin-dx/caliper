// Runs in Storybook's docgen worker thread (`experimental_docgenProvider`). For a story file it resolves
// `meta.component` the way Storybook does, finds that component in a caliper-docgen session, and answers with argTypes
// converted by React's rules. Anything it can't document goes to the next provider, React's own.

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import type { RdtComponentDoc } from 'caliper-docgen-napi'
import {
  STORY_FILE_TEST_REGEXP,
  createMetaComponentResolver,
  extractDeclaredSubcomponents,
  getComponentIdFromEntry,
  getStoryImportPathFromEntry,
} from 'storybook/internal/common'
import { type CsfFile, extractComponentDescription, extractDescription, loadCsf } from 'storybook/internal/csf-tools'
import { logger } from 'storybook/internal/node-logger'
import type { DocgenMiddleware, DocgenPayload, IndexEntry } from 'storybook/internal/types'
import { type ExtractArgTypes, loadExtractArgTypes, toArgTypes } from './arg-types.js'
import type { CaliperStorybookOptions } from './index.js'
import { channel, type Link, type ToDevServer, type ToProvider } from './live.js'
import { DocgenSession, type SessionBinding } from './session.js'
import { assertValidOptions } from './validate.js'

type Subcomponent = NonNullable<DocgenPayload['subcomponents']>[string]

/**
 * How long a component answered with its structural props waits for the checker's answer to replace them. The checker
 * answers a story someone opened before any background work, so only a checker still starting (seconds on a large
 * project) comes near this; past it, the next request asks again.
 */
const CHECKER_WAIT_MS = 60_000

interface StoryFile {
  csf: CsfFile
  storyPath: string
  importPath: string
}

/**
 * `csf`'s story file as the resolver reads it, with `meta.component` being `expression`. The file is parsed again with
 * the expression appended as a statement, so every node has source text for the resolver to report an expression it
 * cannot follow, and every statement stays: the resolver follows the story's imports and local bindings
 * (`const parts = { Item }`) as it does for the written meta, whichever form the meta takes (`export default`,
 * `export { meta as default }`, a CSF factory's `preview.meta()`).
 */
function storyFor(csf: CsfFile, storyPath: string, expression: string): CsfFile {
  const code = `${csf.editor.toString()}\n;(${expression})\n`
  const parsed = loadCsf(code, { fileName: storyPath, makeTitle: () => 'caliper-docgen' }).parse()
  const statement = parsed.program.body.at(-1)
  if (statement?.type !== 'ExpressionStatement') throw new Error(`no expression to resolve for ${expression}`)
  // The parsed file stands in for the written one, with only its component swapped.
  return Object.create(parsed, { _metaAnnotations: { value: { component: statement.expression } } })
}

/** The resolver threw instead of answering: a defect to hand to the next provider, not a fact about one subcomponent. */
class ResolverFailureError extends Error {
  override name = 'ResolverFailureError'
}

/** caliper keeps one value per JSDoc tag; Storybook lists every value. */
function tagLists(tags: Record<string, string>): Record<string, string[]> | undefined {
  const entries = Object.entries(tags)
  return entries.length > 0 ? Object.fromEntries(entries.map(([tag, value]) => [tag, [value]])) : undefined
}

/** Says there is no component `name` in `path`, and which ones the file does have. */
function noComponent(session: DocgenSession, name: string, path: string): string {
  const names = session.namesIn(path)
  const choice = names.length > 0 ? `; its components are ${names.join(', ')}` : ''
  return `caliper-docgen found no component ${name} in ${path}${choice}`
}

export function createDocgenProvider(
  options: CaliperStorybookOptions = {},
  binding?: SessionBinding,
  link: Link<ToDevServer, ToProvider> = channel(),
): DocgenMiddleware {
  assertValidOptions(options)
  const cwd = process.cwd()
  const sessionOptions = {
    srcDirs: (options.srcDirs ?? ['src']).map((dir) => resolve(cwd, dir)),
    exclude: options.exclude,
    tsconfigPath: options.tsconfigPath && resolve(cwd, options.tsconfigPath),
    skipHtmlProps: options.skipHtmlProps,
    htmlAttributes: options.htmlAttributes,
    cache: options.cache,
    // Richer than Storybook's own docgen by default: caliper-docgen is fast enough to read the dependencies' types.
    loadDependencies: options.loadDependencies ?? true,
    collapseInheritedProps: options.collapseInheritedProps ?? 'none',
    lazy: options.lazy,
    checker: options.checker,
    semanticWaitMs: options.semanticWaitMs,
    semanticWaitEmptyMs: options.semanticWaitEmptyMs,
  }
  // The native binding loads on the first story request, not when Storybook starts the worker.
  let sessionPromise: Promise<DocgenSession> | undefined
  async function getSession(): Promise<DocgenSession> {
    sessionPromise ??= (async () =>
      new DocgenSession(sessionOptions, binding ?? (await import('caliper-docgen-napi'))))()
    const attempt = sessionPromise
    try {
      return await attempt
    } catch (error) {
      // A binding that failed to load is tried again by the next request, not remembered as failed.
      if (sessionPromise === attempt) sessionPromise = undefined
      throw error
    }
  }
  // A session that cannot start is reported once at a level a developer sees, then every story falls back quietly.
  let startFailureReported = false

  let started = false
  let refreshFailureReported = false

  /**
   * The session, up to date with the files. A failure before the first successful `fresh()` is a failure to start;
   * after it, a failure is a refresh of a running session and is worded as one. Each is reported once until a
   * `fresh()` succeeds again, and thrown for the fallback either way.
   */
  async function freshSession(): Promise<DocgenSession> {
    try {
      const session = await getSession()
      await session.fresh()
      started = true
      startFailureReported = false
      refreshFailureReported = false
      return session
    } catch (error) {
      const { message } = error as Error
      if (!started && !startFailureReported) {
        startFailureReported = true
        logger.warn(`[caliper-docgen] could not start: ${message}; Storybook's own docgen answers instead`)
      } else if (started && !refreshFailureReported) {
        refreshFailureReported = true
        logger.warn(`[caliper-docgen] could not refresh: ${message}; Storybook's own docgen answers instead`)
      }
      throw error
    }
  }
  const metaComponent = createMetaComponentResolver()
  const resolveMeta = (csf: CsfFile, storyPath: string) => {
    try {
      return metaComponent(csf, storyPath)
    } catch (error) {
      throw new ResolverFailureError((error as Error).message)
    }
  }
  const fallback = options.fallback ?? 'degraded'
  let extractArgTypes: Promise<ExtractArgTypes> | undefined
  // A component handed on is reported once, not on every story request that names it.
  const reported = new Set<string>()

  function reportFallback(key: string, reason: string): void {
    if (reported.has(key)) return
    reported.add(key)
    logger.warn(`[caliper-docgen] ${key}: ${reason}; Storybook's own docgen answers instead`)
  }

  /** The session key and file `target`'s component points at, or an error that says what the session had instead. */
  function locate(session: DocgenSession, target: CsfFile, storyPath: string): { key: string; path: string } {
    const resolution = resolveMeta(target, storyPath)
    if ('component' in resolution) {
      const { path, exportName, localName } = resolution.component
      if (!path) throw new Error(`no file for the import of ${exportName}`)
      const key = session.resolveExport(path, exportName, localName)
      if (key) return { key, path }
      // Storybook follows `Select.Option` into the module that declares `Option` and answers with that local name, which
      // caliper may know as `Select.Option`.
      const member = memberComponent(session, target, storyPath)
      if (member) return member
      throw new Error(noComponent(session, exportName, path))
    }
    // A member Storybook cannot follow into a module's source (`Select.Option` of a named import from a barrel).
    const member = memberComponent(session, target, storyPath)
    if (member) return member
    throw new Error(`meta.component: ${resolution.reason}`)
  }

  /** The sub-component a `Namespace.Member` expression names, when `Namespace` is a named or default import. */
  function memberComponent(
    session: DocgenSession,
    csf: CsfFile,
    storyPath: string,
  ): { key: string; path: string } | undefined {
    const node = csf._metaAnnotations.component as
      | {
          type?: string
          computed?: boolean
          object?: { type?: string; name?: string }
          property?: { type?: string; name?: string }
        }
      | undefined
    if (node?.type !== 'MemberExpression' || node.computed) return undefined
    if (node.object?.type !== 'Identifier' || !node.object.name || !node.property?.name) return undefined
    const owner = resolveMeta(storyFor(csf, storyPath, node.object.name), storyPath)
    if (!('component' in owner) || !owner.component.path) return undefined
    const { path, exportName, localName } = owner.component
    const key = session.keyOfMember({ exportName, localName }, node.property.name, path)
    return key ? { key, path } : undefined
  }

  /**
   * The session key, file and documentation of the component `name` (default: `meta.component`) names, or why there
   * is none. The caller has brought the session up to date for this request. A component answered with structural
   * props because the checker had not answered in time goes in `awaiting`, whether it is answered or handed on.
   */
  async function component(
    csf: CsfFile,
    storyPath: string,
    awaiting: string[],
    name?: string,
  ): Promise<{ key: string; path: string; doc: RdtComponentDoc }> {
    const target = name ? storyFor(csf, storyPath, name) : csf
    const session = await getSession()
    const { key, path } = locate(session, target, storyPath)
    const { doc, degradedBy, awaitsChecker } = session.documentation(key)
    if (awaitsChecker) awaiting.push(key)
    if (fallback === 'degraded' && degradedBy.length > 0) {
      const reason = degradedBy.join('; ')
      reportFallback(key, reason)
      throw new Error(`${key}'s props may be incomplete: ${reason}`)
    }
    if (!doc) throw new Error(noComponent(session, key, path))
    return { key, path, doc }
  }

  async function argTypes(doc: RdtComponentDoc): Promise<DocgenPayload['argTypes']> {
    extractArgTypes ??= loadExtractArgTypes()
    return toArgTypes(doc, await extractArgTypes)
  }

  async function subcomponent(
    { csf, storyPath, importPath }: StoryFile,
    { name, componentName }: { name: string; componentName: string },
    awaiting: string[],
  ): Promise<Subcomponent> {
    try {
      const { path, doc } = await component(csf, storyPath, awaiting, componentName)
      const { description, summary, jsDocTags } = extractComponentDescription(
        undefined,
        doc.description || undefined,
        tagLists(doc.tags),
      )
      return { name, path, description, summary, jsDocTags, argTypes: await argTypes(doc) }
    } catch (error) {
      if (error instanceof ResolverFailureError) throw error
      return {
        name,
        path: importPath,
        jsDocTags: {},
        error: { name: 'No component import found', message: `${componentName}: ${(error as Error).message}` },
      }
    }
  }

  async function storyFile(entry: IndexEntry, importPath: string): Promise<StoryFile> {
    const storyPath = resolve(cwd, importPath)
    const code = await readFile(storyPath, 'utf8')
    const csf = loadCsf(code, { fileName: storyPath, makeTitle: () => entry.title }).parse()
    return { csf, storyPath, importPath }
  }

  async function document(entry: IndexEntry, importPath: string, awaiting: string[]): Promise<DocgenPayload> {
    const file = await storyFile(entry, importPath)
    const { csf, storyPath } = file
    // One look at the files for the story and its subcomponents: they are all answered from the same moment.
    await freshSession()
    const { doc } = await component(csf, storyPath, awaiting)
    const { description, summary, jsDocTags } = extractComponentDescription(
      extractDescription(csf._metaStatement, csf.editor) || undefined,
      doc.description || undefined,
      tagLists(doc.tags),
    )
    const declared = extractDeclaredSubcomponents(csf)
    const subcomponents = Object.fromEntries(
      await Promise.all(
        declared.map(async (declaration) => [declaration.name, await subcomponent(file, declaration, awaiting)]),
      ),
    )
    return {
      id: getComponentIdFromEntry(entry),
      name: csf._meta?.component ?? entry.title.split('/').at(-1)!.replaceAll(/\s+/g, ''),
      path: importPath,
      description,
      summary,
      jsDocTags,
      argTypes: await argTypes(doc),
      ...(declared.length > 0 ? { subcomponents } : {}),
    }
  }

  // Replacing a structural answer (ADR 0022). An answer that went out with structural props because the checker had
  // not answered in time is watched until the checker has answered every component in it; then the dev server, if the
  // browser shows that component, has the docgen service ask again, and this provider answers from the checker's kept
  // answer. Said once per component and revision of the files, so an answer the checker could not improve (its answer
  // refused for an edit made meanwhile) is not asked for again until the files change.

  /** The components being watched. */
  const watching = new Set<string>()
  /** The revision each component's structural answer was last said to be replaced at. */
  const replaced = new Map<string, number>()

  async function watch(componentId: string, awaiting: string[]): Promise<void> {
    if (awaiting.length === 0 || watching.has(componentId)) return
    watching.add(componentId)
    try {
      const session = await getSession()
      const { revision } = session
      if (replaced.get(componentId) === revision) return
      if (!(await session.checkerSettled(awaiting, CHECKER_WAIT_MS))) return
      // An edit since the answer drops the checker's answers; the request that sees the edit answers it.
      if (session.revision !== revision) return
      replaced.set(componentId, revision)
      link.post({ type: 'answered', componentId })
    } catch (error) {
      logger.debug(`[caliper-docgen] ${componentId}: ${(error as Error).message}; the next request asks again`)
    } finally {
      watching.delete(componentId)
    }
  }

  // The prefetch beyond the opening story: the dev server names the stories after the one on screen, and their
  // components are resolved in the background, one story at a time, the checker's answers included. A newer list
  // replaces what is still queued.
  let queue: IndexEntry[] = []
  let prefetching = false

  async function drain(): Promise<void> {
    if (prefetching) return
    prefetching = true
    try {
      while (queue.length > 0) {
        const entry = queue.shift()!
        try {
          // oxlint-disable-next-line no-await-in-loop -- one story at a time, so a story someone opens waits behind one at most
          await prefetchStory(entry)
        } catch (error) {
          logger.debug(`[caliper-docgen] prefetch of ${entry.importPath} stopped: ${(error as Error).message}`)
        }
      }
    } finally {
      prefetching = false
    }
  }

  /** Resolves the components `entry`'s story file names in the background: `meta.component` and its subcomponents. */
  async function prefetchStory(entry: IndexEntry): Promise<void> {
    const importPath = getStoryImportPathFromEntry(entry)
    if (!importPath || !STORY_FILE_TEST_REGEXP.test(importPath)) return
    const { csf, storyPath } = await storyFile(entry, importPath)
    await freshSession()
    const session = await getSession()
    const names = [undefined, ...extractDeclaredSubcomponents(csf).map(({ componentName }) => componentName)]
    const keys = names.flatMap((name) => {
      try {
        return [locate(session, name ? storyFor(csf, storyPath, name) : csf, storyPath).key]
      } catch {
        return []
      }
    })
    await session.prefetch(keys)
  }

  link.listen((message) => {
    // Every end of the channel hears every other, another provider's word included.
    if (message.type !== 'prefetch') return
    queue = [...message.entries]
    void drain()
  })

  return (next) => async (input) => {
    const importPath = getStoryImportPathFromEntry(input.entry)
    if (!importPath || !STORY_FILE_TEST_REGEXP.test(importPath)) return next(input)
    const awaiting: string[] = []
    try {
      return await document(input.entry, importPath, awaiting)
    } catch (error) {
      logger.debug(`[caliper-docgen] ${importPath}: ${(error as Error).message}; asking the next docgen provider`)
      return next(input)
    } finally {
      void watch(getComponentIdFromEntry(input.entry), awaiting)
    }
  }
}
