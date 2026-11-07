// The scorer is pure, so its counts are tested on a small universe: one component with four props the checker finds, and
// a second component a tool may not report at all.
import { describe, expect, it } from 'vitest'
import { prim, type Norm } from '../src/oracle-compare.ts'
import { scoreReported, scoreTool, type Reported, type UniverseProp } from '../src/tool-score.ts'
import { lit } from './lit.ts'

const union = (...members: Norm[]): Norm => ({ k: 'union', members })

const universe: UniverseProp[] = [
  { component: 'a.tsx\0Button', prop: 'size', stratum: 'same_file', checker: union(lit('sm'), lit('lg')) },
  { component: 'a.tsx\0Button', prop: 'label', stratum: 'same_file', checker: prim('string') },
  { component: 'a.tsx\0Button', prop: 'onClick', stratum: 'dependency', checker: prim('number') },
  { component: 'a.tsx\0Button', prop: 'tone', stratum: 'other_project_file', checker: prim('boolean') },
  { component: 'b.tsx\0Card', prop: 'title', stratum: 'same_file', checker: prim('string') },
]

/** What a tool reports, by `component/prop`. */
const tool = (reports: Record<string, Reported>) => (u: UniverseProp) =>
  reports[`${u.component.split('\0')[1]}/${u.prop}`] ?? 'component'

describe('scoreTool', () => {
  it('counts a verdict per listed prop and the rest as missing, by stratum and reason', () => {
    expect.hasAssertions()
    const score = scoreTool(
      universe,
      tool({
        'Button/size': union(lit('sm'), lit('lg')),
        'Button/label': prim('number'),
        'Button/onClick': 'prop',
        'Button/tone': 'prop',
      }),
    )

    expect(score.examples.wrong).toStrictEqual([
      { component: 'a.tsx Button', prop: 'label', tool: prim('number'), checker: prim('string') },
    ])
    expect(score).toMatchObject({
      universe: 5,
      listed: 2,
      missing: 3,
      verdicts: { identical: 1, spelled: 0, 'less precise': 0, wrong: 1, 'checker any/error': 0 },
      agree_universe: 0.2,
      agree_listed: 0.5,
      wrong_listed: 0.5,
      missing_by_stratum: { same_file: 1, other_project_file: 1, dependency: 1, unknown: 0 },
      missing_by_reason: { component: 1, failed_file: 0, prop: 2, collapsed: 0 },
      components: 2,
      components_found: 1,
    })
  })

  it('counts a component the tool threw on separately from one it never reported', () => {
    expect.hasAssertions()
    const score = scoreTool(universe, (u) => (u.component.endsWith('Card') ? 'failed_file' : 'component'))

    expect(score.missing_by_reason).toStrictEqual({ component: 4, failed_file: 1, prop: 0, collapsed: 0 })
    expect(score.listed).toBe(0)
    expect(score.components_found).toBe(0)
    expect(score.agree_universe).toBe(0)
  })

  it('counts a prop in a collapsed layer apart: missing, but not an omission by stratum', () => {
    expect.hasAssertions()
    const score = scoreTool(universe, (u) => (u.prop === 'onClick' ? 'collapsed' : 'prop'))

    expect(score.missing_by_reason).toStrictEqual({ component: 0, failed_file: 0, prop: 4, collapsed: 1 })
    expect(score.missing_by_stratum).toStrictEqual({ same_file: 3, other_project_file: 1, dependency: 0, unknown: 0 })
    expect(score.missing).toBe(5)
    expect(score.components_found).toBe(2)
  })

  it('scores an empty universe as zero, not NaN', () => {
    expect.hasAssertions()
    expect(scoreTool([], () => 'component')).toMatchObject({ universe: 0, agree_universe: 0, agree_listed: 0 })
  })
})

describe('scoreReported', () => {
  it('leaves out the props of a component the tool does not report, and keeps every other verdict and reason', () => {
    expect.hasAssertions()
    // Card is not reported: its prop is not Button's omission, so the universe is Button's four props.
    const score = scoreReported(
      universe,
      tool({
        'Button/size': union(lit('sm'), lit('lg')),
        'Button/label': prim('number'),
        'Button/onClick': 'collapsed',
        'Button/tone': 'prop',
      }),
    )

    expect(score).toMatchObject({
      universe: 4,
      listed: 2,
      verdicts: { identical: 1, spelled: 0, 'less precise': 0, wrong: 1, 'checker any/error': 0 },
      missing_by_reason: { component: 0, failed_file: 0, prop: 1, collapsed: 1 },
      components: 1,
      components_found: 1,
    })
  })

  it('is scoreTool over the whole universe when the tool reports every component', () => {
    expect.hasAssertions()
    const pick = (u: UniverseProp): Reported => (u.prop === 'title' ? prim('string') : 'prop')

    expect(scoreReported(universe, pick)).toStrictEqual(scoreTool(universe, pick))
  })

  it('is empty when the tool reports nothing', () => {
    expect.hasAssertions()
    expect(scoreReported(universe, () => 'component')).toMatchObject({ universe: 0, listed: 0, components: 0 })
  })
})
