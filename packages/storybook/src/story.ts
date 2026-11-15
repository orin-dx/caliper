// Reading a story file the way Storybook's meta-component resolver does, for the docgen provider.

import { type CsfFile, loadCsf } from 'storybook/internal/csf-tools'
import type { DocgenSession } from './session.js'

export interface StoryFile {
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
export function storyFor(csf: CsfFile, storyPath: string, expression: string): CsfFile {
  const code = `${csf.editor.toString()}\n;(${expression})\n`
  const parsed = loadCsf(code, { fileName: storyPath, makeTitle: () => 'caliper-docgen' }).parse()
  const statement = parsed.program.body.at(-1)
  if (statement?.type !== 'ExpressionStatement') throw new Error(`no expression to resolve for ${expression}`)
  // The parsed file stands in for the written one, with only its component swapped.
  return Object.create(parsed, { _metaAnnotations: { value: { component: statement.expression } } })
}

/** The resolver threw instead of answering: a defect to hand to the next provider, not a fact about one subcomponent. */
export class ResolverFailureError extends Error {
  override name = 'ResolverFailureError'
}

/** caliper keeps one value per JSDoc tag; Storybook lists every value. */
export function tagLists(tags: Record<string, string>): Record<string, string[]> | undefined {
  const entries = Object.entries(tags)
  return entries.length > 0 ? Object.fromEntries(entries.map(([tag, value]) => [tag, [value]])) : undefined
}

/** Says there is no component `name` in `path`, and which ones the file does have. */
export function noComponent(session: DocgenSession, name: string, path: string): string {
  const names = session.namesIn(path)
  const choice = names.length > 0 ? `; its components are ${names.join(', ')}` : ''
  return `caliper-docgen found no component ${name} in ${path}${choice}`
}
