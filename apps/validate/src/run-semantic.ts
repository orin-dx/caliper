import { API, SignatureKind, SymbolFlags, TypeFlags } from 'typescript-next/unstable/async'
import { SyntaxKind } from 'typescript-next/unstable/ast'
import { discoverFixtures, FIXTURES_ROOT } from './fixtures.ts'
import type { ToolResult, NormalizedProp } from './types.ts'

const tsconfigPath = `${FIXTURES_ROOT}/../tsconfig.json`

// Mirrors RDT's algorithm via tsc --api instead of RDT itself — see
// docs/type-checker-integration.md, "The algorithm".
const api = new API({ cwd: FIXTURES_ROOT })
const parsed = await api.parseConfigFile(tsconfigPath)

const fixtures = discoverFixtures().filter((f) => !f.isDts) // same filter as the RDT baseline
const program = await api.createProgram(
  fixtures.map((f) => f.path),
  parsed.options,
)
const { checker } = program.project

// A component signature's first parameter is named `props`, or is the only parameter.
async function findComponentSignature(signatures: any[]): Promise<{ param: any } | undefined> {
  const paramsList = await Promise.all(signatures.map((sig) => sig.getParameters()))
  const index = paramsList.findIndex((params) => params.length === 1 || params[0]?.name === 'props')
  return index === -1 ? undefined : { param: paramsList[index][0] }
}

// Class component: props come off the instance type's `props` member.
async function findClassComponentProps(type: any): Promise<{ param: any; propsType: any } | undefined> {
  const constructSignatures = await checker.getSignaturesOfType(type, SignatureKind.Construct)
  const paramsList = await Promise.all(constructSignatures.map((sig: any) => sig.getParameters()))
  const candidates = constructSignatures.filter((_: any, i: number) => paramsList[i].length <= 1)
  const instanceTypes = await Promise.all(candidates.map((sig: any) => sig.getReturnType()))
  const propsSymbols = await Promise.all(instanceTypes.map((t: any) => checker.getPropertyOfType(t, 'props')))
  const index = propsSymbols.findIndex((s: any) => s !== undefined)
  if (index === -1) return undefined
  const propsSymbol = propsSymbols[index]
  return { param: propsSymbol, propsType: await checker.getTypeOfSymbol(propsSymbol) }
}

// parent/declarations: each declaration's parent, when it's an interface or type alias.
async function declaringTypeNames(prop: any): Promise<string[]> {
  const nodes = await Promise.all(prop.declarations.map((d: any) => d.resolve()))
  return nodes
    .map((node: any) => node?.parent)
    .filter((p: any) => p && (p.kind === SyntaxKind.InterfaceDeclaration || p.kind === SyntaxKind.TypeAliasDeclaration))
    .map((p: any) => p.name.getText())
}

// Props: apparent properties, plus each union member's own (getApparentProperties on a
// union already intersects — this recovers props not shared across every branch).
async function collectProps(propsType: any): Promise<Map<string, any>> {
  const byName = new Map<string, any>()
  for (const p of await propsType.getApparentProperties()) byName.set(p.name, p)
  if (propsType.flags & TypeFlags.Union) {
    const members = (await propsType.getTypes()) ?? []
    const memberProps = await Promise.all(members.map((m: any) => m.getApparentProperties()))
    for (const props of memberProps) for (const p of props) if (!byName.has(p.name)) byName.set(p.name, p)
  }
  return byName
}

// RDT's `shouldExtractLiteralValuesFromEnum` prints a literal union (incl. through a type
// alias) as its member values, not the alias name — matches Evidence's "type strings" check.
async function literalUnionString(type: any): Promise<string | undefined> {
  if (!(type.flags & TypeFlags.Union)) return undefined
  const members = (await type.getTypes()) ?? []
  if (members.length === 0 || !members.every((m: any) => m.flags & (TypeFlags.StringLiteral | TypeFlags.NumberLiteral)))
    return undefined
  return members.map((m: any) => (typeof m.value === 'string' ? JSON.stringify(m.value) : String(m.value))).join(' | ')
}

async function normalizeProp(prop: any, location: any): Promise<NormalizedProp> {
  const type = await checker.getTypeOfSymbolAtLocation(prop, location)
  const base = await checker.getBaseConstraintOfType(type)
  const typeStr = (await literalUnionString(base ?? type)) ?? (await checker.typeToString(base ?? type))
  const declarations = await declaringTypeNames(prop)
  return {
    name: prop.name,
    required: !(prop.flags & SymbolFlags.Optional),
    type: typeStr,
    description: await prop.getDocumentationComment(checker),
    parent: declarations[0],
    declarations: declarations.length > 0 ? declarations : undefined,
  }
}

async function processExport(exp: any, fixtureName: string): Promise<ToolResult | undefined> {
  const type = await checker.getTypeOfSymbol(exp)
  const callSignatures = await checker.getSignaturesOfType(type, SignatureKind.Call)
  const callMatch = await findComponentSignature(callSignatures)
  const match = callMatch ?? (await findClassComponentProps(type))
  if (!match) return undefined

  const location = match.param.valueDeclaration ? await match.param.valueDeclaration.resolve() : undefined
  const propsType =
    'propsType' in match ? match.propsType : await checker.getTypeOfSymbolAtLocation(match.param, location)
  const propsByName = await collectProps(propsType)
  const props = Object.fromEntries(
    await Promise.all([...propsByName.values()].map(async (p) => [p.name, await normalizeProp(p, location)] as const)),
  )

  return {
    tool: 'semantic',
    fixture: `${fixtureName}/${exp.name}`,
    durationMs: 0,
    output: {
      [exp.name]: { displayName: exp.name, description: await exp.getDocumentationComment(checker), props },
    },
  }
}

async function processFixture(fixture: { name: string; path: string }): Promise<ToolResult[]> {
  const start = performance.now()
  try {
    const moduleSymbol = await checker.getSymbolOfSourceFile(fixture.path)
    if (!moduleSymbol) {
      return [{ tool: 'semantic', fixture: fixture.name, durationMs: 0, output: {}, error: 'no module symbol' }]
    }

    const exports = await checker.getExportsOfModule(moduleSymbol)
    const exportResults = await Promise.all(exports.map((exp: any) => processExport(exp, fixture.name)))
    const componentResults = exportResults.filter((r): r is ToolResult => r !== undefined)
    const durationMs = performance.now() - start
    for (const r of componentResults) r.durationMs = durationMs

    if (componentResults.length === 0) {
      return [{ tool: 'semantic', fixture: fixture.name, durationMs, output: {}, error: 'no components found' }]
    }
    return componentResults
  } catch (error: any) {
    return [
      {
        tool: 'semantic',
        fixture: fixture.name,
        durationMs: performance.now() - start,
        output: {},
        error: error.message,
      },
    ]
  }
}

const fixtureResults = await Promise.all(fixtures.map((f) => processFixture(f)))
const results = fixtureResults.flat()

await program.dispose()
await api.close()

console.log(JSON.stringify(results, null, 2))
