import { readFileSync, existsSync } from 'node:fs'
import chalk from 'chalk'
import type { ToolResult } from './types.ts'

// Parity check: does run-semantic.ts's tsc-api algorithm reproduce real RDT's output?
// See docs/type-checker-integration.md, "Evidence". Not a quality comparison — that's
// compare.ts/analyze.ts, which weigh oxc-react-docgen against RDT.

function loadBaseline(name: string): Map<string, ToolResult> {
  const p = `./baselines/${name}.json`
  if (!existsSync(p)) {
    console.error(`${chalk.red(`Missing ./baselines/${name}.json`)} — run \`pnpm baseline\` first.`)
    process.exit(1)
  }
  const results: ToolResult[] = JSON.parse(readFileSync(p, 'utf8'))
  return new Map(results.map((r) => [r.fixture, r]))
}

// RDT's own output already ran through its `.replace(' | undefined', '')` twice (lossy —
// turns `=> string | undefined` into `=> string`). Our canonical algorithm keeps
// `| undefined`, so apply the same quirk here rather than "fixing" RDT's output.
function applyRdtUndefinedQuirk(s: string): string {
  return s.replace(' | undefined', '').replace(' | undefined', '')
}

// tsc prints union members in internal type-id order, which differs from RDT's TS
// version — compare as a set instead of a verbatim string.
function typesMatch(oursRawType: string, rdtType: string): boolean {
  const oursType = applyRdtUndefinedQuirk(oursRawType)
  if (oursType === rdtType) return true
  const a = oursType.split('|').map((m) => m.trim())
  const b = rdtType.split('|').map((m) => m.trim())
  return a.length > 1 && a.length === b.length && new Set(a).size === new Set([...a, ...b]).size
}

const rdt = loadBaseline('react-docgen-typescript')
const semantic = loadBaseline('semantic')

let componentsCompared = 0
let propsUnion = 0
let missing = 0
let extra = 0
let requiredMatched = 0
let requiredTotal = 0
let typeMatched = 0
let typeTotal = 0
let descriptionMatched = 0
let descriptionTotal = 0
const mismatches: string[] = []

for (const [key, rdtResult] of rdt) {
  const semanticResult = semantic.get(key)
  if (!semanticResult || rdtResult.error || semanticResult.error) continue

  const [rdtComp] = Object.values(rdtResult.output)
  const [semanticComp] = Object.values(semanticResult.output)
  if (!rdtComp || !semanticComp) continue

  componentsCompared++
  descriptionTotal++
  if (rdtComp.description.trim() === semanticComp.description.trim()) descriptionMatched++
  else mismatches.push(`${key}: description differs`)

  const allPropNames = new Set([...Object.keys(rdtComp.props), ...Object.keys(semanticComp.props)])
  for (const propName of allPropNames) {
    const rp = rdtComp.props[propName]
    const sp = semanticComp.props[propName]
    propsUnion++

    if (!sp) {
      missing++
      mismatches.push(`${key}.${propName}: missing (rdt has it, semantic doesn't)`)
      continue
    }
    if (!rp) {
      extra++
      mismatches.push(`${key}.${propName}: extra (semantic has it, rdt doesn't)`)
      continue
    }

    requiredTotal++
    if (rp.required === sp.required) requiredMatched++
    else mismatches.push(`${key}.${propName}: required rdt=${rp.required} semantic=${sp.required}`)

    typeTotal++
    if (typesMatch(sp.type, rp.type)) typeMatched++
    else mismatches.push(`${key}.${propName}: type rdt="${rp.type}" semantic="${sp.type}"`)
  }
}

function colorForRatio(pct: number, line: string): string {
  if (pct === 100) return chalk.green(line)
  if (pct >= 95) return chalk.yellow(line)
  return chalk.red(line)
}

function ratio(label: string, ok: number, total: number): string {
  const pct = total > 0 ? (ok / total) * 100 : 100
  return colorForRatio(pct, `  ${label}: ${ok}/${total} (${pct.toFixed(1)}%)`)
}

console.log(chalk.bold('Semantic vs RDT parity'))
console.log(`  components compared: ${componentsCompared}`)
console.log(`  props in union: ${propsUnion}`)
console.log(missing === 0 ? chalk.green(`  missing: ${missing}`) : chalk.red(`  missing: ${missing}`))
console.log(extra === 0 ? chalk.green(`  extra: ${extra}`) : chalk.yellow(`  extra: ${extra}`))
console.log(ratio('required', requiredMatched, requiredTotal))
console.log(ratio('type strings', typeMatched, typeTotal))
console.log(ratio('descriptions', descriptionMatched, descriptionTotal))

if (mismatches.length > 0) {
  console.log(chalk.dim(`\n${mismatches.length} mismatches (first 50):`))
  for (const m of mismatches.slice(0, 50)) console.log(chalk.dim(`  ${m}`))
  if (mismatches.length > 50) console.log(chalk.dim(`  ... and ${mismatches.length - 50} more`))
}
