// The checker side of the independent reference: the exported components of one corpus, found the way
// src/oracle.ts finds them, and each own prop of each as a `Candidate` (src/reference-sample.ts) with what is cheap to
// know about it. Nothing here judges a type.
import { realpathSync } from 'node:fs'
import { relative } from 'node:path'
import { ObjectFlags, SignatureKind, SymbolFlags, TypeFlags } from 'typescript-beta/sync'
import {
  keyOf,
  seeded,
  shuffled,
  type Candidate,
  type Origin,
  type PropsShape,
  type ValueKind,
} from './reference-sample.ts'

/** A prop the checker found, with what judging it needs. */
export interface Entry {
  candidate: Candidate
  symbol: any
  type: any
  /** Named by the list the run was given, not drawn for the control sample. */
  listed: boolean
}

export interface Universe {
  checker: any
  project: any
  entries: Entry[]
  /** Exported components with at least one prop, and how many props the collected entries cover. */
  components: number
}

export interface CollectOptions {
  checker: any
  project: any
  corpus: string
  root: string
  /** Keys (`keyOf`) of the props the run was told to judge. */
  wanted: Set<string>
  /** Stop adding control-sample candidates once this many are collected; listed props are always kept. */
  cap: number
  seed: string
  /** Whether a component is one a reader sees. The checker takes any one-parameter function for a component (a style
   * helper, a context), so with a tool's output given the control sample is drawn from the components the tool reports.
   * Listed props are kept either way. */
  accept?: (file: string, component: string) => boolean
}

/** Names the checker finds for a component, matching src/oracle.ts's `normComponent`. */
export const normComponent = (name: string) =>
  (
    name
      .replace(/ \(.*\)$/, '')
      .split('/')
      .at(-1) ?? ''
  )
    .split('.')
    .at(-1)!
    .replaceAll(/[^A-Za-z0-9]/g, '')

const lower = (path: string) => path.toLowerCase()
const GENERIC = TypeFlags.TypeParameter | TypeFlags.IndexedAccess | TypeFlags.Conditional | TypeFlags.Substitution
const LITERAL = TypeFlags.StringLiteral | TypeFlags.NumberLiteral | TypeFlags.BooleanLiteral | TypeFlags.EnumLiteral
const UTILITIES = new Set(['Pick', 'Omit', 'Partial', 'Required', 'Readonly', 'Record'])
// Per component, so one component's hundreds of DOM attributes do not stand for the corpus.
const HTML_PER_COMPONENT = 3

/** The props type of a component export, the way react-docgen-typescript finds it (src/oracle.ts `propsTypeOf`). */
function propsTypeOf(checker: any, exported: any): any | undefined {
  const symbol = exported.flags & SymbolFlags.Alias ? checker.getAliasedSymbol(exported) : exported
  const type = checker.getTypeOfSymbol(symbol)
  for (const sig of checker.getSignaturesOfType(type, SignatureKind.Call)) {
    const params = sig.getParameters()
    if (params.length === 1 || params[0]?.name === 'props') return checker.getTypeOfSymbol(params[0])
  }
  for (const sig of checker.getSignaturesOfType(type, SignatureKind.Construct)) {
    if (sig.getParameters().length > 1) continue
    const props = checker.getPropertyOfType(sig.getReturnType(), 'props')
    if (props) return checker.getTypeOfSymbol(props)
  }
  return undefined
}

/** A default export is named by what it exports. */
function exportName(checker: any, exported: any): string {
  if (exported.name !== 'default') return exported.name
  if (exported.flags & SymbolFlags.Alias) return checker.getAliasedSymbol(exported).name
  return checker.getTypeOfSymbol(exported).getSymbol()?.name ?? 'default'
}

function shapeOf(propsType: any): PropsShape {
  if (propsType.flags & TypeFlags.Intersection) return 'intersection'
  const alias = propsType.getAliasSymbol()?.name
  const mapped = propsType.flags & TypeFlags.Object && propsType.objectFlags & ObjectFlags.Mapped
  return mapped || (alias && UTILITIES.has(alias)) ? 'mapped' : 'plain'
}

function originOf(declaration: string | undefined, file: string): Origin {
  if (declaration === undefined) return 'own'
  const path = lower(declaration)
  if (path.includes('/@types/react/')) return 'html'
  if (path.includes('/node_modules/')) return 'dependency'
  return path === lower(file) ? 'own' : 'extends'
}

/** What a prop's type is, from the checker's flags and, for a union or object, its members' call signatures. */
function kindOf(checker: any, type: any): ValueKind {
  if (type.flags & GENERIC) return 'generic'
  const members: any[] = type.flags & TypeFlags.Union ? (type.getTypes() ?? []) : [type]
  if (members.some((m) => m.flags & GENERIC)) return 'generic'
  if (
    members.some((m) => m.flags & TypeFlags.Object && checker.getSignaturesOfType(m, SignatureKind.Call).length > 0)
  ) {
    return 'function'
  }
  return members.filter((m) => m.flags & LITERAL).length > 1 ? 'union_literal' : 'other'
}

/** Every own prop of the components in `files` as an entry. Files with listed props come first and are always read;
 * the rest, in a seeded order, until `cap` control candidates are collected. */
export function collect(files: string[], options: CollectOptions): Universe {
  const { checker, project } = options
  const root = realpathSync(options.root)
  const rel = (path: string) => relative(root, realpathSync(path))
  const random = seeded(options.seed)
  const withListed = new Set([...options.wanted].map((k) => k.split('\u0000')[0]))
  const ordered = [
    ...files.filter((f) => withListed.has(rel(f))),
    ...shuffled(
      files.filter((f) => !withListed.has(rel(f))),
      random,
    ),
  ]
  const entries: Entry[] = []
  let components = 0
  let control = 0
  for (const file of ordered) {
    const mustRead = withListed.has(rel(file))
    if (!mustRead && control >= options.cap) continue
    const moduleSymbol = checker.getSymbolOfSourceFile(file)
    if (!moduleSymbol) continue
    for (const exported of checker.getExportsOfModule(moduleSymbol)) {
      let propsType
      try {
        propsType = propsTypeOf(checker, exported)
      } catch {
        continue
      }
      if (!propsType) continue
      const props = checker.getPropertiesOfType(propsType).filter((p: any) => p.name !== 'ref' && p.name !== 'key')
      if (props.length === 0) continue
      const name = exportName(checker, exported)
      const accepted = options.accept?.(rel(file), name) ?? true
      if (!accepted && !mustRead) continue
      if (accepted) components++
      const shape = shapeOf(propsType)
      const types = checker.getTypeOfSymbol(props)
      let html = 0
      for (const [i, p] of props.entries()) {
        const origin = originOf(p.declarations?.[0]?.path ? String(p.declarations[0].path) : undefined, file)
        const candidate: Candidate = {
          corpus: options.corpus,
          file: rel(file),
          component: name,
          prop: p.name,
          origin,
          shape,
          kind: kindOf(checker, types[i]),
        }
        const listed = options.wanted.has(keyOf({ ...candidate, component: normComponent(name) }))
        if (!listed && !accepted) continue
        if (!listed && origin === 'html' && ++html > HTML_PER_COMPONENT) continue
        entries.push({ candidate, symbol: p, type: types[i], listed })
        if (!listed) control++
      }
    }
  }
  return { checker, project, entries, components }
}
