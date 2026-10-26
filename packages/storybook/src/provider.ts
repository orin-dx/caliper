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
import { DocgenSession, type SessionBinding } from './session.js'

type Subcomponent = NonNullable<DocgenPayload['subcomponents']>[string]

interface StoryFile {
  csf: CsfFile
  storyPath: string
  importPath: string
}

function identifier(name: string): object {
  return { type: 'Identifier', name }
}

/** A component expression node for a dotted or plain name, the shape the meta-component resolver reads. */
function expressionFor(name: string): object {
  const [object, property] = name.split('.')
  return property
    ? { type: 'MemberExpression', computed: false, object: identifier(object!), property: identifier(property) }
    : identifier(name)
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
): DocgenMiddleware {
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
    warmUp: options.warmUp,
    fillAfter: options.fillAfter,
  }
  // The native binding loads on the first story request, not when Storybook starts the worker.
  let sessionPromise: Promise<DocgenSession> | undefined
  const getSession = () =>
    (sessionPromise ??= (async () =>
      new DocgenSession(sessionOptions, binding ?? (await import('caliper-docgen-napi'))))())
  const resolveMeta = createMetaComponentResolver()
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
  async function locate(
    session: DocgenSession,
    target: CsfFile,
    storyPath: string,
  ): Promise<{ key: string; path: string }> {
    const resolution = resolveMeta(target, storyPath)
    if ('component' in resolution) {
      const { path, exportName, localName } = resolution.component
      if (!path) throw new Error(`no file for the import of ${exportName}`)
      const key = await session.resolveExport(path, exportName, localName)
      if (key) return { key, path }
      throw new Error(noComponent(session, exportName, path))
    }
    // Storybook reads `Namespace.Member` only for `import * as Namespace`; `Select.Option` of a named import is common.
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
    const owner = resolveMeta(
      { _metaAnnotations: { component: node.object }, _file: csf._file } as unknown as CsfFile,
      storyPath,
    )
    if (!('component' in owner) || !owner.component.path) return undefined
    const { path, exportName, localName } = owner.component
    const key = session.keyOfMember([localName, exportName], node.property.name, path)
    return key ? { key, path } : undefined
  }

  /**
   * The session key, file and documentation of the component `name` (default: `meta.component`) names, or why there
   * is none. The caller has brought the session up to date for this request.
   */
  async function component(
    csf: CsfFile,
    storyPath: string,
    name?: string,
  ): Promise<{ key: string; path: string; doc: RdtComponentDoc }> {
    const target = name
      ? ({ _metaAnnotations: { component: expressionFor(name) }, _file: csf._file } as unknown as CsfFile)
      : csf
    const session = await getSession()
    const { key, path } = await locate(session, target, storyPath)
    const { doc, degradedBy } = session.documentation(key)
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
  ): Promise<Subcomponent> {
    try {
      const { path, doc } = await component(csf, storyPath, componentName)
      const { description, summary, jsDocTags } = extractComponentDescription(
        undefined,
        doc.description || undefined,
        tagLists(doc.tags),
      )
      return { name, path, description, summary, jsDocTags, argTypes: await argTypes(doc) }
    } catch (error) {
      return {
        name,
        path: importPath,
        jsDocTags: {},
        error: { name: 'No component import found', message: `${componentName}: ${(error as Error).message}` },
      }
    }
  }

  async function document(entry: IndexEntry, importPath: string): Promise<DocgenPayload> {
    const storyPath = resolve(cwd, importPath)
    const code = await readFile(storyPath, 'utf8')
    const csf = loadCsf(code, { fileName: storyPath, makeTitle: () => entry.title }).parse()
    // One look at the files for the story and its subcomponents: they are all answered from the same moment.
    const session = await getSession()
    await session.fresh()
    const { doc } = await component(csf, storyPath)
    const { description, summary, jsDocTags } = extractComponentDescription(
      extractDescription(csf._metaStatement) || undefined,
      doc.description || undefined,
      tagLists(doc.tags),
    )
    const declared = extractDeclaredSubcomponents(csf)
    const subcomponents = Object.fromEntries(
      await Promise.all(
        declared.map(async (declaration) => [
          declaration.name,
          await subcomponent({ csf, storyPath, importPath }, declaration),
        ]),
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

  return (next) => async (input) => {
    const importPath = getStoryImportPathFromEntry(input.entry)
    if (!importPath || !STORY_FILE_TEST_REGEXP.test(importPath)) return next(input)
    try {
      return await document(input.entry, importPath)
    } catch (error) {
      logger.debug(`[caliper-docgen] ${importPath}: ${(error as Error).message}; asking the next docgen provider`)
      return next(input)
    }
  }
}
