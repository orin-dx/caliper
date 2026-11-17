// The 2026-11-09 weekly's react-spectrum universe, from the saved dump of the oracle's universe (5,582 props, aggregated
// by component in tests/fixtures/spectrum-universe-2026-11-09.json): what whole-universe agreement reads with every
// function counted, and with the functions that are no component left out.
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { nonComponentReason, type ShapeProp } from '../src/component-shape.ts'
import { prim } from '../src/oracle-compare.ts'
import { scoreTool, type Reported, type UniverseProp } from '../src/tool-score.ts'

type Row = [name: string, file: string, props: number, caliper: number, rdt: number, lib: number]
const fixture: { rows: Row[] } = JSON.parse(
  readFileSync(join(import.meta.dirname, 'fixtures', 'spectrum-universe-2026-11-09.json'), 'utf8'),
)

interface Entry extends UniverseProp {
  caliper: Reported
  rdt: Reported
  /** What the shape rules say of the function, over the props the dump kept: the ones `@types/react` declares are not in it. */
  reason: string | undefined
}

const LIB = '/node_modules/@typescript/typescript-darwin-arm64/lib/lib.es5.d.ts'

/** One universe prop per saved prop: the tool agrees on the first `agrees` of a component's props and omits the rest. */
function expand(): Entry[] {
  const entries: Entry[] = []
  for (const [name, file, props, caliper, rdt, lib] of fixture.rows) {
    const declared = new Map<string, ShapeProp>(
      Array.from({ length: props }, (_, i) => [`p${i}`, { path: i < lib ? LIB : '/r/a.ts', type: () => 'string' }]),
    )
    // The dump names 529 default exports `default`, which the shape rules would read as a lowercase helper: the oracle
    // names them now, so they are not asked here.
    const reason = name === 'default' ? undefined : nonComponentReason(name, `/r/${file}`, declared)
    for (let i = 0; i < props; i++) {
      entries.push({
        component: `${file}\0${name}`,
        prop: `p${i}`,
        stratum: 'same_file',
        checker: prim('string'),
        caliper: i < caliper ? prim('string') : 'prop',
        rdt: i < rdt ? prim('string') : 'prop',
        reason,
      })
    }
  }
  return entries
}

const percent = (entries: Entry[], tool: 'caliper' | 'rdt') =>
  100 * scoreTool(entries, (u) => (u as Entry)[tool]).agree_universe

// adjusted.py of the diagnosis: a hook (`use` and a capital) or a lowercase name is no component, and `default` is the
// name the oracle gave an export it could not name.
const helper = (name: string) => /^use[A-Z0-9]/.test(name) || /^\p{Ll}/u.test(name)
const nameOf = (e: Entry) => e.component.split('\0')[1]

describe('the react-spectrum universe of 2026-11-09', () => {
  const entries = expand()

  it('has 5,582 props, and Caliper agrees on less of them than RDT when every function counts', () => {
    expect(entries).toHaveLength(5582)
    expect(percent(entries, 'caliper')).toBeCloseTo(30.7, 1)
    expect(percent(entries, 'rdt')).toBeCloseTo(42.5, 1)
  })

  it('has the two tools level once lowercase and hook functions are left out', () => {
    const components = entries.filter((e) => !helper(nameOf(e)) || nameOf(e) === 'default')

    expect(components).toHaveLength(4708)
    expect(percent(components, 'caliper')).toBeCloseTo(36.4, 1)
    expect(percent(components, 'rdt')).toBeCloseTo(35.9, 1)
  })

  it('has them at 81.8% and 80.6% on the props whose name was never in doubt', () => {
    const named = entries.filter((e) => !helper(nameOf(e)) && nameOf(e) !== 'default')

    expect(named).toHaveLength(2095)
    expect(percent(named, 'caliper')).toBeCloseTo(81.8, 1)
    expect(percent(named, 'rdt')).toBeCloseTo(80.6, 1)
  })

  it('has the shape rules take out hooks, lowercase helpers and lib-typed parameters, and nothing that is named', () => {
    const excluded = entries.filter((e) => e.reason !== undefined)
    const byReason = Object.groupBy(excluded, (e) => e.reason!)

    expect(Object.fromEntries(Object.entries(byReason).map(([reason, es]) => [reason, es!.length]))).toStrictEqual({
      lib_members: 730,
      hook: 58,
      not_jsx: 76,
    })
    expect(excluded.every((e) => helper(nameOf(e)))).toBe(true)
  })

  it('has the two tools level over the candidates, 36.3% and 36.0% of 4,718 props, with every default still counted', () => {
    const candidates = entries.filter((e) => e.reason === undefined)

    expect(candidates).toHaveLength(4718)
    expect(percent(candidates, 'caliper')).toBeCloseTo(36.3, 1)
    expect(percent(candidates, 'rdt')).toBeCloseTo(36.0, 1)
  })
})
