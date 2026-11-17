// How the oracles find a component with TypeScript 7.1's checker: the props type of an exported function or class, and
// the names it is exported under. The rule is react-docgen-typescript's (an exported function whose only or `props`
// parameter is the props type), so src/oracle.ts and src/agent-truth.ts must both agree on it; they share
// this file so they cannot drift.
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

/** The symbol at `path` in a module: the export named `path[0]` exactly (`default` for the default export), then each
 * property of it an attached sub-component is reached through. Undefined when any step is missing. */
export function exportAt(checker: any, moduleSymbol: any, path: string[]): any | undefined {
  const [name, ...members] = path
  let symbol = checker.getExportsOfModule(moduleSymbol).find((e: any) => e.name === name)
  for (const member of members) {
    if (!symbol) return undefined
    const resolved = symbol.flags & SymbolFlags.Alias ? checker.getAliasedSymbol(symbol) : symbol
    symbol = checker.getPropertyOfType(checker.getTypeOfSymbol(resolved), member)
  }
  return symbol
}

/** The name a declaration gives itself: `Foo` in `export default function Foo` and in `export default Foo`, undefined for
 * an anonymous function. The checker names the symbol of a default-exported function `default`, and the declaration
 * handle carries no name until it is resolved to its node. */
export function declaredName(checker: any, exported: any): string | undefined {
  const symbol = exported.flags & SymbolFlags.Alias ? checker.getAliasedSymbol(exported) : exported
  const declared = symbol.declarations?.[0]?.resolve()?.name?.text
  if (typeof declared === 'string' && declared !== '') return declared
  return symbol.name === 'default' ? undefined : symbol.name
}

/** One component of a module: a declaration the module makes, and every name the module exports it under. */
export interface DeclaredExport {
  /** The export the props are read through: the one named like the declaration, else the first. */
  exported: any
  /** Every name the module exports the declaration under, `default` included: what Caliper's `exports` lists for it. */
  names: string[]
  /** The name the declaration gives itself, when it has one. */
  declared: string | undefined
}

/** A component is its declaration, not the name it is exported under: `export default function Foo` followed by
 * `export { Foo }` or `export { Foo as Bar }` is one component, and a re-export belongs to the file that declares it. The
 * exports of a module are taken once per declaration made in that module. Whatever counts or compares components takes
 * them from here. */
export function declaredExports(checker: any, moduleSymbol: any): DeclaredExport[] {
  const home = moduleSymbol.declarations?.[0]?.path
  const byDeclaration = new Map<string, any[]>()
  for (const exported of checker.getExportsOfModule(moduleSymbol)) {
    const symbol = exported.flags & SymbolFlags.Alias ? checker.getAliasedSymbol(exported) : exported
    const handle = symbol.declarations?.[0]
    if (!handle || handle.path !== home) continue
    const key = `${handle.path}:${handle.resolve()?.pos}`
    byDeclaration.set(key, [...(byDeclaration.get(key) ?? []), exported])
  }
  return [...byDeclaration.values()].map((exports) => {
    const declared = declaredName(checker, exports[0])
    return {
      exported: exports.find((e) => e.name === declared) ?? exports[0],
      names: exports.map((e) => e.name),
      declared,
    }
  })
}

/** What react-docgen-typescript and react-docgen call a component, which they know only by name: the name it is
 * exported under, and for a default export the declaration's own name, else the file's, or its directory's for an
 * `index`, as react-docgen-typescript does. Only the tools' dumps (src/tool-universe.ts) are matched on it; Caliper's
 * components are matched by their exports. */
export function listedName(exportName: string, declared: string | undefined, file: string): string {
  if (exportName !== 'default') return exportName
  if (declared !== undefined) return declared
  const parts = file.split('/')
  const base = parts.at(-1) ?? ''
  const dot = base.lastIndexOf('.')
  const stem = dot > 0 ? base.slice(0, dot) : base
  return stem === 'index' ? (parts.at(-2) ?? stem) : stem
}
