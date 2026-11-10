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

/** A default export is named by what it exports: the aliased declaration, or the function or class itself. */
export function exportName(checker: any, exported: any): string {
  if (exported.name !== 'default') return exported.name
  if (exported.flags & SymbolFlags.Alias) return checker.getAliasedSymbol(exported).name
  return checker.getTypeOfSymbol(exported).getSymbol()?.name ?? 'default'
}
