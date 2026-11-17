// The shape rules that say an exported function takes no props: its single parameter is a `string` or a `Request`, it is
// a hook, a context's Provider, a helper no JSX tag could name, or a route that is handed promises. The checker has no
// notion of a component, so the oracle's universe (src/tool-universe.ts) uses these to count only candidates.
//
// scripts/weekly.py `exclusion` applies the same rules to RDT's entries for the recall reference. The patterns are in
// src/component-shape.json, and the paths they read (a lib file, `@types/react`) are classed by src/path-classes.json;
// both files are read by both, and both run the cases of tests/fixtures/component-shape-cases.json, so a change to one
// that is not made to the other fails a test.
import { readFileSync } from 'node:fs'
import { isLibFile, isReactTypes } from './path-class.ts'

interface Rules {
  hook: string
  contextValueProps: string[]
  promiseType: string
}

const rules: Rules = JSON.parse(readFileSync(new URL('component-shape.json', import.meta.url), 'utf8'))
const HOOK = new RegExp(rules.hook)
const PROMISE = new RegExp(rules.promiseType)

/** Why a function is no component, in the order the rules are tried. */
export const SHAPE_REASONS = ['lib_members', 'hook', 'context_value', 'not_jsx', 'promise_props'] as const
export type ShapeReason = (typeof SHAPE_REASONS)[number]

/** One prop of a function's parameter. `path` is the file that declares it, `type` its type as source text (read only
 * when a rule needs it). */
export interface ShapeProp {
  path: string | undefined
  type: () => string
}

/** A name as the weekly's `norm_name` spells it: no `(file)` suffix, namespace or punctuation, lowercase. */
export const normName = (name: string): string =>
  (
    name
      .replace(/ \(.*\)$/, '')
      .split('/')
      .at(-1) ?? ''
  )
    .split('.')
    .at(-1)!
    .toLowerCase()
    .replaceAll(/[^a-z0-9]/g, '')

/** The names RDT gives a default export of `file`: the file's own, and its directory's. */
function ownNames(file: string): Set<string> {
  const parts = file.split('/')
  const base = parts.at(-1) ?? ''
  const dot = base.lastIndexOf('.')
  return new Set([normName(dot > 0 ? base.slice(0, dot) : base), normName(parts.at(-2) ?? '')])
}

const isLowercase = (name: string): boolean => {
  const [first] = name
  return first !== undefined && first === first.toLowerCase() && first !== first.toUpperCase()
}

/** Why `name`, exported from `file` with a parameter of these props (every prop, `@types/react`'s too), is not a
 * component; undefined when it may be one. */
export function nonComponentReason(
  name: string,
  file: string,
  props: ReadonlyMap<string, ShapeProp>,
): ShapeReason | undefined {
  const all = [...props.values()]
  const inReact = (p: ShapeProp) => isReactTypes(p.path ?? '')
  if (all.filter((p) => isLibFile(p.path ?? '')).length * 2 > all.length) return 'lib_members'
  if (HOOK.test(name)) return 'hook'
  if (
    props.has('value') &&
    [...props.keys()].every((key) => rules.contextValueProps.includes(key)) &&
    all.every((p) => inReact(p))
  ) {
    return 'context_value'
  }
  if (isLowercase(name) && !ownNames(file).has(normName(name)) && !all.some((p) => inReact(p))) return 'not_jsx'
  if (all.every((p) => PROMISE.test(p.type()))) return 'promise_props'
  return undefined
}
