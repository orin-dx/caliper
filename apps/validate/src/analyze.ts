import { readFileSync, existsSync, writeFileSync } from 'node:fs'
import type { ToolResult, NormalizedProp } from './types.ts'

// ─── Loading ──────────────────────────────────────────────────────────────

function loadBaseline(name: string): Map<string, ToolResult> | null {
  const p = `./baselines/${name}.json`
  if (!existsSync(p)) return null
  const results: ToolResult[] = JSON.parse(readFileSync(p, 'utf8'))
  return new Map(results.map((r) => [r.fixture, r]))
}

const ours = loadBaseline('caliper-docgen')
// react-docgen resolves no inherited/`extends` props of its own — compare against our
// html-attributes=none baseline (falls back to the full baseline if that variant wasn't
// generated) so this pair isn't dominated by an inheritance-resolution mismatch instead of
// an actual prop-level agreement signal.
const oursForRdg = loadBaseline('caliper-docgen-none') ?? ours
const rdt = loadBaseline('react-docgen-typescript')
const rdg = loadBaseline('react-docgen')

if (!ours) {
  console.error('No caliper-docgen baseline found — run `pnpm run:ours` first.')
  process.exit(1)
}

// ─── Divergence taxonomy ──────────────────────────────────────────────────
//
// Classifies a (ours, comparator) type-string mismatch into a bucket by
// observable shape only — it can't see root cause, just the diff. Root
// causes for the recurring buckets are cross-referenced in docs/benchmarks.md
// against docs/rdt-coverage.md's already-investigated gap list.

type TypeDiffBucket =
  | 'cosmetic-formatting'
  | 'literal-narrowed-vs-widened'
  | 'union-member-order'
  | 'optional-undefined-representation'
  | 'structural-type-difference'

function normalizeTypeString(s: string): string {
  return s.replaceAll(/\s+/g, ' ').replaceAll('"', "'").trim()
}

function stripUndefinedMember(s: string): string {
  return s
    .split('|')
    .map((m) => m.trim())
    .filter((m) => m !== 'undefined')
    .join(' | ')
}

function isLiteralish(s: string): boolean {
  return /^['"]/.test(s.trim()) || s.includes('|')
}

const WIDENED = new Set(['string', 'number', 'boolean', 'unknown', 'any'])

function classifyTypeDiff(oursType: string, otherType: string): TypeDiffBucket {
  if (normalizeTypeString(oursType) === normalizeTypeString(otherType)) return 'cosmetic-formatting'

  if (stripUndefinedMember(oursType) === stripUndefinedMember(otherType) && oursType !== otherType) {
    return 'optional-undefined-representation'
  }

  const oursMembers = oursType.split('|').map((m) => m.trim())
  const otherMembers = otherType.split('|').map((m) => m.trim())
  if (
    oursMembers.length > 1 &&
    oursMembers.length === otherMembers.length &&
    new Set(oursMembers).size === oursMembers.length &&
    [...oursMembers].toSorted().join('|') === [...otherMembers].toSorted().join('|') &&
    oursType !== otherType
  ) {
    return 'union-member-order'
  }

  if (
    (WIDENED.has(otherType.trim()) && isLiteralish(oursType)) ||
    (WIDENED.has(oursType.trim()) && isLiteralish(otherType))
  ) {
    return 'literal-narrowed-vs-widened'
  }

  return 'structural-type-difference'
}

// ─── Per-pair agreement computation ──────────────────────────────────────

interface PropDiff {
  fixture: string
  component: string
  prop: string
  kind: 'type-mismatch' | 'required-mismatch' | 'default-mismatch' | 'missing-in-ours' | 'extra-in-ours'
  bucket?: TypeDiffBucket
  ours?: string
  other?: string
  htmlInheritanceCovered?: boolean
}

interface PairSummary {
  comparator: 'react-docgen-typescript' | 'react-docgen'
  componentsCompared: number
  /** Components where the comparator returned zero props at all — relevant for
   *  react-docgen specifically, which has weak native support for interface-typed
   *  (non-PropTypes) TypeScript components and frequently returns nothing. A flat
   *  per-prop agreement rate is close to meaningless when this is high; the more
   *  honest number is "how many real components does each tool find props for at all." */
  componentsWithEmptyComparatorOutput: number
  propsUnion: number
  matched: number
  mismatched: number
  missingInOurs: number
  missingButHtmlInheritanceCovered: number
  extraInOurs: number
  /** matched / (propsUnion - missingButHtmlInheritanceCovered) — the honest denominator excludes
   *  props we intentionally surface a different way (curated HTML-attribute inheritance) rather
   *  than pretending those never existed. */
  agreementRate: number
  typeDiffBuckets: Record<TypeDiffBucket, number>
  diffs: PropDiff[]
}

function comparePair(
  oursMap: Map<string, ToolResult>,
  otherMap: Map<string, ToolResult>,
  comparatorName: 'react-docgen-typescript' | 'react-docgen',
): PairSummary {
  const summary: PairSummary = {
    comparator: comparatorName,
    componentsCompared: 0,
    componentsWithEmptyComparatorOutput: 0,
    propsUnion: 0,
    matched: 0,
    mismatched: 0,
    missingInOurs: 0,
    missingButHtmlInheritanceCovered: 0,
    extraInOurs: 0,
    agreementRate: 0,
    typeDiffBuckets: {
      'cosmetic-formatting': 0,
      'literal-narrowed-vs-widened': 0,
      'union-member-order': 0,
      'optional-undefined-representation': 0,
      'structural-type-difference': 0,
    },
    diffs: [],
  }

  for (const [key, ourResult] of oursMap) {
    const otherResult = otherMap.get(key)
    if (!otherResult || ourResult.error || otherResult.error) continue

    const fixture = key.split('/').slice(0, -1).join('/')
    const component = key.split('/').at(-1) ?? key
    const [ourComp] = Object.values(ourResult.output)
    const [otherComp] = Object.values(otherResult.output)
    if (!ourComp || !otherComp) continue

    summary.componentsCompared++
    if (Object.keys(otherComp.props).length === 0) summary.componentsWithEmptyComparatorOutput++
    const notableInherited = new Set(ourResult.notableInheritedNames)
    const allPropNames = new Set([...Object.keys(ourComp.props), ...Object.keys(otherComp.props)])

    for (const propName of allPropNames) {
      const op: NormalizedProp | undefined = ourComp.props[propName]
      const cp: NormalizedProp | undefined = otherComp.props[propName]

      if (op && !cp) {
        summary.propsUnion++
        summary.extraInOurs++
        summary.diffs.push({ fixture, component, prop: propName, kind: 'extra-in-ours', ours: op.type })
        continue
      }
      if (!op && cp) {
        summary.propsUnion++
        if (notableInherited.has(propName)) {
          summary.missingButHtmlInheritanceCovered++
          summary.diffs.push({
            fixture,
            component,
            prop: propName,
            kind: 'missing-in-ours',
            other: cp.type,
            htmlInheritanceCovered: true,
          })
        } else {
          summary.missingInOurs++
          summary.diffs.push({ fixture, component, prop: propName, kind: 'missing-in-ours', other: cp.type })
        }
        continue
      }
      if (!op || !cp) continue // unreachable, both branches above cover the single-sided cases

      summary.propsUnion++
      if (op.required !== cp.required) {
        summary.mismatched++
        summary.diffs.push({
          fixture,
          component,
          prop: propName,
          kind: 'required-mismatch',
          ours: String(op.required),
          other: String(cp.required),
        })
      } else if (op.type !== cp.type) {
        const bucket = classifyTypeDiff(op.type, cp.type)
        summary.typeDiffBuckets[bucket]++
        if (bucket === 'cosmetic-formatting') {
          summary.matched++
        } else {
          summary.mismatched++
          summary.diffs.push({
            fixture,
            component,
            prop: propName,
            kind: 'type-mismatch',
            bucket,
            ours: op.type,
            other: cp.type,
          })
        }
      } else if ((op.defaultValue ?? null) === (cp.defaultValue ?? null)) {
        summary.matched++
      } else {
        summary.mismatched++
        summary.diffs.push({
          fixture,
          component,
          prop: propName,
          kind: 'default-mismatch',
          ours: op.defaultValue,
          other: cp.defaultValue,
        })
      }
    }
  }

  const honestDenominator = summary.propsUnion - summary.missingButHtmlInheritanceCovered
  summary.agreementRate = honestDenominator > 0 ? summary.matched / honestDenominator : 1

  return summary
}

// ─── Run ──────────────────────────────────────────────────────────────────

const results: { generatedAt: string; pairs: PairSummary[] } = {
  generatedAt: new Date().toISOString(),
  pairs: [],
}

if (rdt) results.pairs.push(comparePair(ours, rdt, 'react-docgen-typescript'))
if (rdg) results.pairs.push(comparePair(oursForRdg, rdg, 'react-docgen'))

writeFileSync('./analysis.json', JSON.stringify(results, null, 2))

for (const pair of results.pairs) {
  console.log(`\n${'='.repeat(70)}`)
  console.log(`ours vs ${pair.comparator}`)
  console.log('='.repeat(70))
  console.log(`Components compared: ${pair.componentsCompared}`)
  if (pair.componentsWithEmptyComparatorOutput > 0) {
    console.log(
      `  ⚠️  ${pair.comparator} returned ZERO props for ${pair.componentsWithEmptyComparatorOutput}/${pair.componentsCompared} of these components — a per-prop agreement rate is a weak signal when this is high; see docs/benchmarks.md.`,
    )
  }
  console.log(`Props in union:      ${pair.propsUnion}`)
  console.log(`  matched:                          ${pair.matched}`)
  console.log(`  mismatched (real):                ${pair.mismatched}`)
  console.log(`  missing in ours (real miss):       ${pair.missingInOurs}`)
  console.log(`  missing in ours (HTML-inherited):  ${pair.missingButHtmlInheritanceCovered}`)
  console.log(`  extra in ours:                     ${pair.extraInOurs}`)
  console.log(`Agreement rate (excl. HTML-inheritance design difference): ${(pair.agreementRate * 100).toFixed(1)}%`)
  console.log('Type-diff taxonomy (excluding cosmetic-formatting, which counts as matched):')
  for (const [bucket, count] of Object.entries(pair.typeDiffBuckets)) {
    if (bucket !== 'cosmetic-formatting' && count > 0) console.log(`  ${bucket}: ${count}`)
  }
}

console.log(`\nFull structured output written to apps/validate/analysis.json`)
