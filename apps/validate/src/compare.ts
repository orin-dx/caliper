import { readFileSync, existsSync } from 'node:fs'
import type { ToolResult } from './types.ts'

function loadBaseline(name: string): Map<string, ToolResult> | null {
  const p = `./baselines/${name}.json`
  if (!existsSync(p)) return null
  const results: ToolResult[] = JSON.parse(readFileSync(p, 'utf8'))
  return new Map(results.map((r) => [r.fixture, r]))
}

const rdg = loadBaseline('react-docgen')
const rdt = loadBaseline('react-docgen-typescript')
const ours = loadBaseline('caliper-docgen')

if (!rdg && !rdt && !ours) {
  console.error('No baselines found. Run at least one tool first.')
  process.exit(1)
}

const allFixtures = new Set([
  ...(rdg ? [...rdg.keys()] : []),
  ...(rdt ? [...rdt.keys()] : []),
  ...(ours ? [...ours.keys()] : []),
])

let totalOwn = 0,
  rdtTotal = 0,
  oursTotal = 0,
  wins = 0,
  ties = 0,
  misses = 0

for (const fixture of [...allFixtures].toSorted()) {
  const r_rdg = rdg?.get(fixture)
  const r_rdt = rdt?.get(fixture)
  const r_ours = ours?.get(fixture)

  console.log(`\n## ${fixture}`)

  if (r_rdg?.error) console.log(`  ❌ react-docgen: ${r_rdg.error.slice(0, 100)}`)
  if (r_rdt?.error) console.log(`  ❌ react-docgen-typescript: ${r_rdt.error.slice(0, 100)}`)
  if (r_ours?.error) console.log(`  ❌ caliper-docgen: ${r_ours.error.slice(0, 100)}`)

  const times: string[] = []
  if (r_rdg) times.push(`rdg: ${r_rdg.durationMs.toFixed(1)}ms`)
  if (r_rdt) times.push(`rdt: ${r_rdt.durationMs.toFixed(1)}ms`)
  if (r_ours) times.push(`ours: ${r_ours.durationMs.toFixed(1)}ms`)
  if (times.length > 0) console.log(`  ⏱  ${times.join('  |  ')}`)

  const rdgComps = Object.keys(r_rdg?.output ?? {})
  const rdtComps = Object.keys(r_rdt?.output ?? {})
  const oursComps = Object.keys(r_ours?.output ?? {})
  const allComps = new Set([...rdgComps, ...rdtComps, ...oursComps])

  for (const comp of [...allComps].toSorted()) {
    const rdgProps = Object.keys(r_rdg?.output?.[comp]?.props ?? {})
    const rdtProps = Object.keys(r_rdt?.output?.[comp]?.props ?? {})
    const oursProps = Object.keys(r_ours?.output?.[comp]?.props ?? {})

    console.log(`\n  ### ${comp}`)
    console.log(`    props: rdg=${rdgProps.length}  rdt=${rdtProps.length}  ours=${oursProps.length}`)

    rdtTotal += rdtProps.length
    oursTotal += oursProps.length

    // Props ours has that rdt doesn't
    const oursOnly = oursProps.filter((p) => !rdtProps.includes(p))
    const rdtOnly = rdtProps.filter((p) => !oursProps.includes(p))
    const rdgOnly = rdgProps.filter((p) => !oursProps.includes(p))
    const common = oursProps.filter((p) => rdtProps.includes(p))
    const ourInheritedElements = r_ours?.inheritedElements ?? []

    if (oursOnly.length > 0) console.log(`    ours-only (${oursOnly.length}): ${oursOnly.slice(0, 8).join(', ')}`)
    if (rdgOnly.length > 0)
      console.log(
        `    rdg-only  (${rdgOnly.length}): ${rdgOnly.slice(0, 5).join(', ')}${rdgOnly.length > 5 ? '...' : ''}`,
      )

    // For rdt-only props: distinguish HTML attrs covered by our notableInherited from true misses.
    // We trust our notableInherited list as the ground truth for what we surface from HTML elements.
    const ourNotableNames = new Set(r_ours?.notableInheritedNames)
    if (rdtOnly.length > 0) {
      // A prop is "covered by our inheritance" if it appears in our notableInherited list.
      // Props not in notableInherited and not in our own props are true misses.
      const coveredByInheritance = rdtOnly.filter((p) => ourNotableNames.has(p))
      const trueMisses = rdtOnly.filter((p) => !ourNotableNames.has(p) && !oursProps.includes(p))
      if (trueMisses.length > 0) {
        console.log(`    ❗ rdt-only REAL MISSES (${trueMisses.length}): ${trueMisses.slice(0, 8).join(', ')}`)
        misses += trueMisses.length
      }
      if (coveredByInheritance.length > 0) {
        console.log(`    ℹ️  rdt expands ${coveredByInheritance.length} attrs we surface via notableInherited`)
      }
      const unexpandedHtmlAttrs = rdtOnly.filter(
        (p) => !ourNotableNames.has(p) && !oursProps.includes(p) && /^(on[A-Z]|aria-|data-)/.test(p),
      )
      if (unexpandedHtmlAttrs.length > 0 && ourInheritedElements.length > 0) {
        console.log(
          `    ℹ️  ${unexpandedHtmlAttrs.length} additional HTML event/ARIA attrs rdt expands (not in our notableInherited)`,
        )
      }
    }

    // Type comparison (ours vs rdt)
    const typeDiffs = common
      .filter((p) => {
        const o = r_ours?.output?.[comp]?.props?.[p]?.type
        const t = r_rdt?.output?.[comp]?.props?.[p]?.type
        return o && t && o !== t
      })
      .map((p) => ({
        prop: p,
        ours: r_ours?.output?.[comp]?.props?.[p]?.type ?? '?',
        rdt: r_rdt?.output?.[comp]?.props?.[p]?.type ?? '?',
      }))

    if (typeDiffs.length > 0) {
      console.log(`    type diffs (${typeDiffs.length}):`)
      for (const d of typeDiffs.slice(0, 5)) {
        console.log(`      ${d.prop}: ours="${d.ours}"  rdt="${d.rdt}"`)
      }
    }

    // A win is: no real misses, no unexpected extra props, no type diffs, and we found something.
    const rdtOnlyRealCount = rdtOnly.filter((p) => !ourNotableNames.has(p) && !oursProps.includes(p)).length

    if (oursOnly.length === 0 && rdtOnlyRealCount === 0 && typeDiffs.length === 0 && oursProps.length > 0) {
      console.log(`    ✅ own props match rdt`)
      wins++
    } else if (oursProps.length === 0 && rdtProps.length === 0) {
      ties++
    } else if (oursProps.length === 0 && rdtProps.length > 0 && ourInheritedElements.length > 0) {
      const notableCount = ourNotableNames.size
      const rdtCovered = rdtOnly.filter((p) => ourNotableNames.has(p)).length
      if (rdtCovered === rdtProps.length) {
        console.log(`    ✅ 0 own props, all ${rdtProps.length} rdt props covered by notableInherited`)
        wins++
      } else {
        console.log(
          `    ⚠️  0 own props; we surface ${notableCount} notableInherited, rdt has ${rdtProps.length} total (inherits ${ourInheritedElements.join(', ')})`,
        )
      }
    } else if (oursProps.length === 0 && rdtProps.length > 0 && ourInheritedElements.length === 0) {
      console.log(`    ❌ ours found nothing, rdt found ${rdtProps.length} props`)
    }

    totalOwn += oursProps.length
  }
}

console.log(`\n${'='.repeat(60)}`)
console.log('SUMMARY')
console.log('='.repeat(60))
if (rdt) console.log(`react-docgen-typescript: ${rdtTotal} total props across all components`)
if (ours) console.log(`caliper-docgen:        ${oursTotal} total props across all components`)
console.log(`Coverage: ${oursTotal}/${rdtTotal} (${Math.round((oursTotal / Math.max(rdtTotal, 1)) * 100)}%)`)
console.log(`Identical matches: ${wins}  |  Both empty: ${ties}  |  Misses: ${misses}`)
