// The tsrs-against-Go parity run (docs/adr/0026-the-checker-is-the-type-authority.md, construct-suite/README.md): the
// one-program capture of src/construct-capture.ts, once with the Go checker (typescript-beta) and once with tsrs, both
// normalized by src/oracle-checker.ts and compared row by row. A row differs when the normalized type, the
// `typeToString` text, or the facts Caliper reads off the type through the API (`flags`, `intrinsicName`) differ.
// `pnpm --filter caliper-docgen-oracle construct:parity` prints the differences and exits non-zero on any that
// construct-suite/parity-allowlist.json does not list.
import { existsSync, readFileSync, realpathSync } from 'node:fs'
import { join } from 'node:path'
import { exportName, propsTypeOf } from './checker-components.ts'
import { listFixtures, PROP } from './construct-suite.ts'
import { checkerNormalizer } from './oracle-checker.ts'
import type { Norm } from './oracle-compare.ts'

/** What one checker says about a row's prop `p`. */
export interface RowFacts {
  /** The type, normalized by src/oracle-checker.ts. */
  norm: Norm
  /** `checker.typeToString` of the type. */
  text: string
  /** `type.flags`: with `intrinsicName`, what separates the error type from `any`. */
  flags: number
  intrinsicName: string | null
}

/** The facts of every row, by row id. */
export type Capture = Record<string, RowFacts>

export type FactName = 'norm' | 'text' | 'flags' | 'intrinsicName'

export interface RowDifference {
  id: string
  /** The facts that differ, in the order `FactName` lists them. */
  facts: FactName[]
  go: RowFacts
  tsrs: RowFacts
}

export interface ParityResult {
  rows: number
  identical: number
  /** Differences the allow-list does not cover: the run fails on these. */
  unexpected: RowDifference[]
  /** Differences the allow-list covers, with the reason it gives. */
  allowed: { difference: RowDifference; reason: string }[]
  /** Allow-listed rows that no longer differ, or are not rows: the list must stay honest. */
  staleAllowed: { id: string; reason: string }[]
  /** Rows only one side captured. */
  missing: string[]
}

export type Allowlist = Record<string, string>

// A unique-symbol key is named `__@tag@<symbol id>`; the id depends on the program (src/construct-capture.ts).
const stableSymbolIds = (json: string) => json.replaceAll(/__@(\w+)@\d+/g, '__@$1')
const canonical = (norm: Norm) => stableSymbolIds(JSON.stringify(norm, null, 2))

const FACTS: FactName[] = ['norm', 'text', 'flags', 'intrinsicName']

/** The facts that differ between two checkers' answers for one row. */
export function differingFacts(go: RowFacts, tsrs: RowFacts): FactName[] {
  return FACTS.filter((fact) => {
    if (fact === 'norm') return canonical(go.norm) !== canonical(tsrs.norm)
    if (fact === 'text') return stableSymbolIds(go.text) !== stableSymbolIds(tsrs.text)
    return go[fact] !== tsrs[fact]
  })
}

/** Compares two captures under an allow-list. Pure: neither checker runs here. */
export function compareCaptures(go: Capture, tsrs: Capture, allowlist: Allowlist): ParityResult {
  const ids = [...new Set([...Object.keys(go), ...Object.keys(tsrs)])].toSorted()
  const result: ParityResult = {
    rows: ids.length,
    identical: 0,
    unexpected: [],
    allowed: [],
    staleAllowed: [],
    missing: [],
  }
  const differing = new Set<string>()
  for (const id of ids) {
    const [g, t] = [go[id], tsrs[id]]
    if (g === undefined || t === undefined) {
      result.missing.push(`${id}: only ${g === undefined ? 'tsrs' : 'the Go checker'} captured it`)
      continue
    }
    const facts = differingFacts(g, t)
    if (facts.length === 0) {
      result.identical += 1
      continue
    }
    differing.add(id)
    const difference: RowDifference = { id, facts, go: g, tsrs: t }
    const reason = allowlist[id]
    if (reason === undefined) result.unexpected.push(difference)
    else result.allowed.push({ difference, reason })
  }
  for (const [id, reason] of Object.entries(allowlist)) {
    if (!differing.has(id)) result.staleAllowed.push({ id, reason })
  }
  return result
}

/** A line diff of two texts (`-` Go, `+` tsrs), by longest common subsequence. */
export function lineDiff(a: string, b: string): string {
  const [x, y] = [a.split('\n'), b.split('\n')]
  const lcs = Array.from({ length: x.length + 1 }, () => Array.from<number>({ length: y.length + 1 }).fill(0))
  for (let i = x.length - 1; i >= 0; i--) {
    for (let j = y.length - 1; j >= 0; j--) {
      lcs[i]![j] = x[i] === y[j] ? lcs[i + 1]![j + 1]! + 1 : Math.max(lcs[i + 1]![j]!, lcs[i]![j + 1]!)
    }
  }
  const out: string[] = []
  let [i, j] = [0, 0]
  while (i < x.length || j < y.length) {
    if (i < x.length && j < y.length && x[i] === y[j]) {
      out.push(`  ${x[i]}`)
      i++
      j++
    } else if (j >= y.length || (i < x.length && lcs[i + 1]![j]! >= lcs[i]![j + 1]!)) {
      out.push(`- ${x[i]}`)
      i++
    } else {
      out.push(`+ ${y[j]}`)
      j++
    }
  }
  return out.join('\n')
}

function describeDifference({ id, facts, go, tsrs }: RowDifference): string {
  const lines = [`${id}: differs in ${facts.join(', ')}`, `  go:   ${go.text}`, `  tsrs: ${tsrs.text}`]
  if (facts.includes('flags') || facts.includes('intrinsicName')) {
    lines.push(
      `  go flags ${go.flags} intrinsicName ${String(go.intrinsicName)}; tsrs flags ${tsrs.flags} intrinsicName ${String(tsrs.intrinsicName)}`,
    )
  }
  if (facts.includes('norm')) {
    const diff = lineDiff(canonical(go.norm), canonical(tsrs.norm))
    lines.push('  normalized diff (- go, + tsrs):', ...diff.split('\n').map((l) => `    ${l}`))
  }
  return lines.join('\n')
}

/** The text of a run's report. */
export function renderReport(result: ParityResult): string {
  const lines = [
    `${result.rows} rows: ${result.identical} identical, ${result.allowed.length} allow-listed, ` +
      `${result.unexpected.length} unexpected, ${result.staleAllowed.length} stale allow-list, ${result.missing.length} missing`,
  ]
  for (const d of result.unexpected) lines.push('', `UNEXPECTED ${describeDifference(d)}`)
  for (const { difference, reason } of result.allowed) {
    lines.push('', `allowed (${reason}) ${describeDifference(difference)}`)
  }
  for (const { id, reason } of result.staleAllowed) {
    lines.push('', `STALE ALLOW-LIST ${id}: no longer differs (was: ${reason}); remove it from parity-allowlist.json`)
  }
  for (const m of result.missing) lines.push('', `MISSING ${m}`)
  return lines.join('\n')
}

/** True when the run must fail: an unexpected difference, a stale allow-list entry, or a row only one side has. */
export const failed = (r: ParityResult): boolean =>
  r.unexpected.length > 0 || r.staleAllowed.length > 0 || r.missing.length > 0

/** The slice of an `API` module (typescript-beta/sync or tsrs/sync) the capture uses. */
export interface ApiModule {
  API: new (options: { cwd: string }) => any
}

/** The one-program capture of src/construct-capture.ts with the given API: every fixture's `Row.p`. */
export function captureRows(mod: ApiModule, suite: string): Capture {
  const fixtures = listFixtures(suite)
  if (fixtures.length === 0) throw new Error(`no fixtures under ${suite}`)
  const api = new mod.API({ cwd: suite })
  try {
    const { options } = api.parseConfigFile(join(suite, 'tsconfig.json'))
    const program = api.createProgram(
      fixtures.map((f) => f.file),
      options,
    )
    try {
      const { checker } = program.getProject()
      const norm = checkerNormalizer(checker)
      const rows: Capture = {}
      for (const fixture of fixtures) {
        const moduleSymbol = checker.getSymbolOfSourceFile(fixture.file)
        const exported =
          moduleSymbol && checker.getExportsOfModule(moduleSymbol).find((e: any) => exportName(checker, e) === 'Row')
        if (!exported) throw new Error(`${fixture.id}: no exported Row`)
        const propsType = propsTypeOf(checker, exported)
        const prop = propsType && checker.getPropertyOfType(propsType, PROP)
        if (!prop) throw new Error(`${fixture.id}: Row has no prop ${PROP}`)
        const type = checker.getTypeOfSymbol(prop)
        rows[fixture.id] = {
          norm: norm(type),
          text: checker.typeToString(type),
          flags: type.flags,
          intrinsicName: typeof type.intrinsicName === 'string' ? type.intrinsicName : null,
        }
      }
      return rows
    } finally {
      program.dispose()
    }
  } finally {
    api.close()
  }
}

export const suiteDir = (): string => realpathSync(join(import.meta.dirname, '..', 'construct-suite'))

export function readAllowlist(suite: string): Allowlist {
  const file = join(suite, 'parity-allowlist.json')
  return existsSync(file) ? (JSON.parse(readFileSync(file, 'utf8')) as Allowlist) : {}
}

/** The tsrs API module, or undefined when the package is not installed. Any other failure to load it is an error. */
export async function loadTsrs(): Promise<ApiModule | undefined> {
  try {
    return (await import('tsrs/sync')) as ApiModule
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ERR_MODULE_NOT_FOUND') return undefined
    throw error
  }
}

/** The whole run: both captures, compared under the allow-list file. */
export async function runParity(tsrs: ApiModule): Promise<ParityResult> {
  const suite = suiteDir()
  const go = captureRows(await import('typescript-beta/sync'), suite)
  return compareCaptures(go, captureRows(tsrs, suite), readAllowlist(suite))
}

if (process.argv[1] !== undefined && import.meta.filename === realpathSync(process.argv[1])) {
  const tsrs = await loadTsrs()
  if (tsrs === undefined) throw new Error('tsrs is not installed (pnpm install); it is a devDependency of apps/oracle')
  const result = await runParity(tsrs)
  console.error(renderReport(result))
  process.exit(failed(result) ? 1 : 0)
}
