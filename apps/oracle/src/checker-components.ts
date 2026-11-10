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
