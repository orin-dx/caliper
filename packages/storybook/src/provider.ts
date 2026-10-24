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
    // Richer than Storybook's own docgen by default: caliper-docgen is fast enough to read the dependencies' types.
    loadDependencies: options.loadDependencies ?? true,
    collapseInheritedProps: options.collapseInheritedProps ?? 'none',
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

  /** The session key and file of the component `name` (default: `meta.component`) names, or why there is none. */
  async function component(csf: CsfFile, storyPath: string, name?: string): Promise<{ key: string; path: string }> {
    const target = name
      ? ({ _metaAnnotations: { component: expressionFor(name) }, _file: csf._file } as unknown as CsfFile)
      : csf
    const resolution = resolveMeta(target, storyPath)
    if (!('component' in resolution)) throw new Error(`meta.component: ${resolution.reason}`)
    const { path, exportName } = resolution.component
    if (!path) throw new Error(`no file for the import of ${exportName}`)
    const session = await getSession()
    await session.fresh()
    const key = session.keyOf(path, exportName)
    if (!key) throw new Error(`caliper-docgen found no component ${exportName} in ${path}`)
    if (fallback === 'degraded') {
      const reasons = session.degradedBy(key)
      if (reasons.length > 0) {
        const reason = reasons.join('; ')
        reportFallback(key, reason)
        throw new Error(`${key}'s props may be incomplete: ${reason}`)
      }
    }
    return { key, path }
  }

  async function docgen(key: string): Promise<RdtComponentDoc> {
    const session = await getSession()
    const doc = session.docgenInfo(key)
    if (!doc) throw new Error(`${key} left the session`)
    return doc
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
      const { key, path } = await component(csf, storyPath, componentName)
      const doc = await docgen(key)
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
    const { key } = await component(csf, storyPath)
    const doc = await docgen(key)
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
