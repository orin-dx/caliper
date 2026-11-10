// How the oracles find a component with TypeScript 7.1's checker: the props type of an exported function or class, and
// the name it is exported under. The rule is react-docgen-typescript's (an exported function whose only or `props`
// parameter is the props type), so src/oracle.ts, src/agent-truth.ts and src/reference.ts must all agree on it; they
// share this file so they cannot drift.
import { SignatureKind, SymbolFlags } from 'typescript-beta/sync'

/** The props type of a component export, the way react-docgen-typescript finds it. */
export function propsTypeOf(checker: any, exported: any): any | undefined {
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

/** A file's stem in PascalCase, the name Caliper gives a default export that has none of its own
 * (`pascal_case_from_file_stem` in crates/core/src/extractor/component.rs): `button-group.tsx` is `ButtonGroup`. */
function pascalCaseFromFile(file: string): string | undefined {
  const base = file.split('/').at(-1) ?? ''
  const dot = base.lastIndexOf('.')
  const stem = dot > 0 ? base.slice(0, dot) : base
  const pascal = stem
    .split(/[^\p{L}\p{N}]/u)
    .map((segment) => segment.charAt(0).toUpperCase() + segment.slice(1))
    .join('')
  return pascal === '' ? undefined : pascal
}

/** A component is its declaration, not the name it is exported under: `export default function Foo` followed by
 * `export { Foo }` or `export { Foo as Bar }` is one component, and a re-export belongs to the file that declares it. The
 * exports of a module are taken once per declaration made in that module, under the export that carries the
 * declaration's own name, else the first. Whatever counts or compares components takes them from here. */
export function declaredExports(checker: any, moduleSymbol: any): any[] {
  const home = moduleSymbol.declarations?.[0]?.path
  const byDeclaration = new Map<string, { names: Set<string>; exports: any[] }>()
  for (const exported of checker.getExportsOfModule(moduleSymbol)) {
    const symbol = exported.flags & SymbolFlags.Alias ? checker.getAliasedSymbol(exported) : exported
    const handle = symbol.declarations?.[0]
    if (!handle || handle.path !== home) continue
    const key = `${handle.path}:${handle.resolve()?.pos}`
    const group = byDeclaration.get(key) ?? { names: new Set<string>(), exports: [] }
    const declared = handle.resolve()?.name?.text
    if (typeof declared === 'string') group.names.add(declared)
    group.exports.push(exported)
    byDeclaration.set(key, group)
  }
  return [...byDeclaration.values()].map(
    ({ names, exports }) => exports.find((e) => names.has(exportName(checker, e))) ?? exports[0],
  )
}

/** A default export is named by what it exports: the aliased symbol, else the name in its declaration (`export default
 * function Foo`), else the file's, as Caliper does. The checker names the symbol, and the type of a default-exported
 * function, `default`, and the declaration handle carries no name until it is resolved to its node. */
export function exportName(checker: any, exported: any): string {
  if (exported.name !== 'default') return exported.name
  const symbol = exported.flags & SymbolFlags.Alias ? checker.getAliasedSymbol(exported) : exported
  if (symbol.name !== 'default') return symbol.name
  const node = symbol.declarations?.[0]?.resolve()
  const declared = node?.name?.text
  if (typeof declared === 'string' && declared !== '') return declared
  return (node && pascalCaseFromFile(node.getSourceFile().fileName)) ?? 'default'
}
