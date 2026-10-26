// TypeScript 7.1's checker as a test oracle for Caliper's prop types (docs/parity.md). Test-only: the opt-in semantic
// backend is a separate, experimental feature.
//
// For one corpus, finds components the way react-docgen-typescript does (an exported function whose only or `props`
// parameter is the props type), reads each prop's type from the checker, and compares it structurally with the prop
// Caliper's canonical JSON reports for the same component (src/oracle-compare.ts). Each shared prop gets one verdict:
//
//   identical          both sides describe the same type
//   spelled            the same type, Caliper by name where the checker expands it, or the reverse
//   less precise       Caliper's type is looser or unevaluated (an opaque type, `string` for a literal union, a name
//                      left unexpanded, a lost optional or rest parameter, a missing field)
//   wrong              Caliper's type describes something else
//   checker any/error  the checker itself has no type (an unresolved import, `any`, or `unknown` where Caliper has one)
//
// Checker types are normalized once per type id and verdicts made once per (Caliper prop, checker type), so Chakra's
// ~470,000 props cost a few thousand distinct comparisons.
//
// Usage: tsx src/oracle.ts --root DIR --src DIR --tsconfig FILE --caliper caliper.json --out verdicts.json
//                          [--files list.txt] [--dump props.json] [--rows rows.json] [--missing missing.json]
//
// `--dump` writes every prop judged `wrong` or `less precise` (the examples in `--out` stop at 20 per verdict), so two
// builds' verdicts can be diffed prop by prop. `--rows` writes one `[component key, prop, verdict]` row for every shared
// prop, which `scripts/confidence.py` joins with the canonical JSON to ask which signals predict a disagreement.
import { readdirSync, readFileSync, realpathSync, writeFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { API, SignatureKind, SymbolFlags } from 'typescript-beta/sync'
import { checkerNormalizer } from './oracle-checker.ts'
import { caliperNorm, compare, VERDICTS, type Norm, type Verdict } from './oracle-compare.ts'

const KEYS = ['root', 'src', 'tsconfig', 'caliper', 'out', 'files', 'dump', 'rows', 'missing'] as const
const args: Partial<Record<(typeof KEYS)[number], string>> = {}
for (let i = 2; i < process.argv.length; i += 2) {
  args[process.argv[i].replace(/^--/, '') as (typeof KEYS)[number]] = process.argv[i + 1]
}
if (!args.root || !args.src || !args.tsconfig || !args.caliper || !args.out) {
  console.error('usage: tsx src/oracle.ts --root DIR --src DIR --tsconfig FILE --caliper FILE --out FILE')
  process.exit(2)
}
const root = realpathSync(args.root)

function walk(dir: string): string[] {
  const out: string[] = []
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === 'node_modules') continue
    const path = join(dir, entry.name)
    if (entry.isDirectory()) out.push(...walk(path))
    else out.push(path)
  }
  return out
}

const files: string[] = args.files
  ? readFileSync(args.files, 'utf8').split('\n').filter(Boolean)
  : walk(realpathSync(args.src))
      .filter((f) => f.endsWith('.tsx'))
      .toSorted()

const api = new API({ cwd: root })
const programStart = performance.now()
// The same program react-docgen-typescript parses in scripts/parity.py: the component files, with the tsconfig it
// uses. The options go through `parseConfigFile` because the 7.1 beta's server panics on string-valued enum options
// passed to `createProgram` directly.
const { options } = api.parseConfigFile(args.tsconfig)
const program = api.createProgram(files, options)
const { checker } = program.getProject()
const programMs = performance.now() - programStart
const checkerNorm = checkerNormalizer(checker)

// ─── Components ───────────────────────────────────────────────────────────────

const normComponent = (name: string) =>
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
const rel = (path: string) => relative(root, realpathSync(path))
const pathKey = (path: string) =>
  process.platform === 'darwin' || process.platform === 'win32' ? path.toLowerCase() : path

const caliper = JSON.parse(readFileSync(args.caliper, 'utf8'))
const caliperByFile = new Map<string, { name: string; key: string; component: any }[]>()
for (const [componentKey, component] of Object.entries<any>(caliper.components)) {
  // Schema 2 lists a component's prop set ids, each a map of prop name to prop id; this reads one map of both.
  if (Array.isArray(component.props)) {
    component.props = Object.assign({}, ...component.props.map((id: string) => caliper.propSets[id]))
  }
  const key = rel(component.filePath)
  const list = caliperByFile.get(key) ?? []
  list.push({ name: normComponent(component.displayName), key: componentKey, component })
  caliperByFile.set(key, list)
}

/** The props type of a component export, the way react-docgen-typescript finds it. */
function propsTypeOf(exported: any): any | undefined {
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

/** A default export is named by what it exports: the aliased declaration, or the function or class itself. */
function exportName(exported: any): string {
  if (exported.name !== 'default') return exported.name
  if (exported.flags & SymbolFlags.Alias) return checker.getAliasedSymbol(exported).name
  return checker.getTypeOfSymbol(exported).getSymbol()?.name ?? 'default'
}

interface Example {
  component: string
  prop: string
  caliper: unknown
  checker: string
  normalized: object
}
const counts = Object.fromEntries(VERDICTS.map((v) => [v, 0])) as Record<Verdict, number>
const examples = Object.fromEntries(VERDICTS.map((v) => [v, [] as Example[]])) as Record<Verdict, Example[]>
const verdictCache = new Map<string, Verdict>()
const rows: [string, string, Verdict | 'missing' | 'collapsed'][] = []
const missingDetails: { key: string; prop: string; verdict: 'missing' | 'collapsed'; declarations: string[] }[] = []
const dumped: {
  key: string
  file: string
  component: string
  prop: string
  verdict: Verdict
  caliper: unknown
  checker: string
}[] = []
const totals = {
  componentsChecker: 0,
  componentsMatched: 0,
  propsChecker: 0,
  propsShared: 0,
  propsMissing: 0,
  propsCollapsed: 0,
  failedExports: 0,
}

/** The verdict for one prop, made once per (Caliper prop, checker type). */
function judge({
  component,
  prop,
  caliperId,
  type,
}: {
  component: string
  prop: string
  caliperId: string
  type: any
}) {
  const key = `${caliperId}:${type.id}`
  const cached = verdictCache.get(key)
  if (cached !== undefined) return cached
  const c: Norm = caliperNorm(caliper.props[caliperId].type)
  const k: Norm = checkerNorm(type)
  const verdict = compare(c, k)
  verdictCache.set(key, verdict)
  if (examples[verdict].length < 20) {
    const checkerText = checker.typeToString(type)
    examples[verdict].push({
      component,
      prop,
      caliper: caliper.props[caliperId].type,
      checker: checkerText,
      normalized: { caliper: c, checker: k },
    })
  }
  return verdict
}

/** Every prop of one exported component the checker finds, against Caliper's component of the same name. */
function visitExport(exported: any, candidates: { name: string; key: string; component: any }[], file: string): void {
  let propsType
  try {
    propsType = propsTypeOf(exported)
  } catch {
    totals.failedExports++
    return
  }
  if (!propsType) return
  const props = checker.getPropertiesOfType(propsType)
  if (props.length === 0) return
  totals.componentsChecker++
  const name = exportName(exported)
  // React's DOM attributes are one inherited layer; ref and key are deliberately absent from Caliper's output.
  const own = props.filter(
    (p: any) =>
      p.name !== 'ref' &&
      p.name !== 'key' &&
      !p.declarations?.some((d: any) => String(d.path).includes('/@types/react/')),
  )
  totals.propsChecker += own.length
  const match = candidates.find((c) => c.name === normComponent(name))
  if (!match) return
  totals.componentsMatched++
  const shared = own.filter((p: any) => p.name in match.component.props)
  const layerFiles = new Set(
    (match.component.inheritance ?? [])
      .filter((layer: any) => layer.totalProps > 30)
      .map((layer: any) => pathKey(String(layer.fileName))),
  )
  const absent = own.filter((p: any) => !(p.name in match.component.props))
  const collapsed: any[] = []
  const missing: any[] = []
  for (const p of absent) {
    if (p.declarations?.some((d: any) => layerFiles.has(pathKey(String(d.path))))) collapsed.push(p)
    else missing.push(p)
  }
  totals.propsMissing += missing.length
  totals.propsCollapsed += collapsed.length
  if (args.rows) rows.push(...missing.map((p: any) => [match.key, p.name, 'missing'] as [string, string, 'missing']))
  if (args.rows)
    rows.push(...collapsed.map((p: any) => [match.key, p.name, 'collapsed'] as [string, string, 'collapsed']))
  if (args.missing) {
    for (const [verdict, absentProps] of [
      ['missing', missing],
      ['collapsed', collapsed],
    ] as const) {
      for (const p of absentProps) {
        missingDetails.push({
          key: match.key,
          prop: p.name,
          verdict,
          declarations: p.declarations?.map((d: any) => String(d.path)) ?? [],
        })
      }
    }
  }
  if (shared.length === 0) return
  const types = checker.getTypeOfSymbol(shared)
  for (const [i, p] of shared.entries()) {
    totals.propsShared++
    const caliperId = match.component.props[p.name]
    const verdict = judge({ component: name, prop: p.name, caliperId, type: types[i] })
    counts[verdict]++
    if (args.rows) rows.push([match.key, p.name, verdict])
    if (args.dump && (verdict === 'wrong' || verdict === 'less precise')) {
      dumped.push({
        key: match.key,
        file: rel(file),
        component: name,
        prop: p.name,
        verdict,
        caliper: caliper.props[caliperId].type,
        checker: checker.typeToString(types[i]),
      })
    }
  }
}

const start = performance.now()
for (const file of files) {
  const moduleSymbol = checker.getSymbolOfSourceFile(file)
  if (!moduleSymbol) continue
  const candidates = caliperByFile.get(rel(file)) ?? []
  for (const exported of checker.getExportsOfModule(moduleSymbol)) visitExport(exported, candidates, file)
}
const compareMs = performance.now() - start

const report = {
  components_checker: totals.componentsChecker,
  components_matched: totals.componentsMatched,
  props_checker: totals.propsChecker,
  props_shared: totals.propsShared,
  props_missing: totals.propsMissing,
  props_collapsed: totals.propsCollapsed,
  verdicts: counts,
  distinct_comparisons: verdictCache.size,
  failed_exports: totals.failedExports,
  program_ms: Math.round(programMs),
  compare_ms: Math.round(compareMs),
  examples,
}
writeFileSync(args.out, JSON.stringify(report, null, 2))
if (args.dump) writeFileSync(args.dump, JSON.stringify(dumped))
if (args.rows) writeFileSync(args.rows, JSON.stringify(rows))
if (args.missing) writeFileSync(args.missing, JSON.stringify(missingDetails))
program.dispose()
api.close()
console.error(
  `${totals.componentsMatched}/${totals.componentsChecker} components, ${totals.propsShared} shared props, ` +
    `${verdictCache.size} distinct comparisons, program ${(programMs / 1000).toFixed(1)} s, ` +
    `compare ${(compareMs / 1000).toFixed(1)} s`,
)
