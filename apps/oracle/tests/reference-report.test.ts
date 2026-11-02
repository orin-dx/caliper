// The counts the reference reports and the contested props it writes out for a person.
import { describe, expect, it } from 'vitest'
import { informative, reviewText, summarize, type Row } from '../src/reference-report.ts'

function row(prop: string, over: Partial<Row> = {}): Row {
  return {
    corpus: 'antd',
    stratum: 'own',
    origin: 'own',
    file: 'button/Button.tsx',
    component: 'Button',
    prop,
    checker: 'string',
    declared: 'string',
    reference: 'agreed',
    basis: 'both',
    ...over,
  }
}

const rows: Row[] = [
  row('a', { tool: { outcome: 'correct', basis: 'both' } }),
  row('b', {
    stratum: 'html',
    origin: 'html',
    basis: 'checker',
    why: 'abstained',
    abstained: 'no declaration',
    tool: { outcome: 'correct', basis: 'checker' },
  }),
  row('c', { stratum: 'function', tool: { outcome: 'wrong', basis: 'both' } }),
  row('d', {
    stratum: 'generic',
    reference: 'contested',
    basis: undefined,
    declared: 'number',
    pair: 'prim->prim',
    location: '/r/Button.tsx',
    tool: { outcome: 'contested', leans: 'checker' },
  }),
  row('e', {
    reference: 'contested',
    basis: undefined,
    pair: 'name->union',
    declared: 'Mode',
    location: '/r/Mode.d.ts',
  }),
  row('f', { stratum: 'html', origin: 'html', tool: { outcome: 'missing' } }),
]

describe('summarize', () => {
  const s = summarize(rows, true)

  it('counts the reference by the methods that stood behind it', () => {
    expect.hasAssertions()
    expect(s.sampled).toBe(6)
    expect(s.reference).toStrictEqual({ agreed_both: 3, checker_only: 1, declaration_only: 0, contested: 2, none: 0 })
    expect(s.declaration_abstained).toStrictEqual({ 'no declaration': 1 })
    expect(s.checker_alone_because).toStrictEqual({ abstained: 1 })
    expect(s.contested_kinds).toStrictEqual({ 'prim->prim': 1, 'name->union': 1 })
  })

  it('counts a tool by outcome, the methods behind each, and the stratum', () => {
    expect.hasAssertions()
    expect(s.scored.props).toBe(5)
    expect(s.scored.tool_unmatched).toBe(1)
    expect(s.scored.outcomes).toStrictEqual({
      correct: 2,
      'less precise': 0,
      wrong: 1,
      contested: 1,
      'no reference': 0,
      layer: 0,
      missing: 1,
    })
    expect(s.scored.correct_by_basis).toStrictEqual({ both: 1, checker: 1 })
    expect(s.scored.wrong_by_basis).toStrictEqual({ both: 1 })
    expect(s.scored.contested_leans).toStrictEqual({ checker: 1 })
    expect(s.scored.by_stratum.html).toStrictEqual({ correct: 1, missing: 1 })
    expect(s.scored.by_origin).toStrictEqual({
      own: { correct: 1, wrong: 1, contested: 1 },
      html: { correct: 1, missing: 1 },
    })
  })

  it('does not report unmatched props when no tool was given', () => {
    expect.hasAssertions()
    expect(
      summarize(
        rows.map(({ tool: _tool, ...r }) => r),
        false,
      ).scored.tool_unmatched,
    ).toBeUndefined()
  })
})

describe('informative', () => {
  it('puts the contested props the tool settles first', () => {
    expect.hasAssertions()
    expect(informative(rows).map((r) => r.prop)).toStrictEqual(['d', 'e'])
    expect(informative(rows, 1).map((r) => r.prop)).toStrictEqual(['d'])
  })
})

describe('reviewText', () => {
  it('lists only the contested props, with both types and where the declaration is', () => {
    expect.hasAssertions()
    expect(reviewText(rows)).toBe(
      [
        '# Contested props (2)',
        '',
        'The checker (A) and the declaration (B) disagree. Decide which is the type a reader should see.',
        '',
        '## antd: Button.d',
        '',
        '- file: button/Button.tsx',
        '- declaration: /r/Button.tsx',
        '- checker (A): string',
        '- declared (B): number',
        '- tool: contested, agrees with checker',
        '',
        '## antd: Button.e',
        '',
        '- file: button/Button.tsx',
        '- declaration: /r/Mode.d.ts',
        '- checker (A): string',
        '- declared (B): Mode',
        '- tool: not scored',
        '',
      ].join('\n'),
    )
  })
})
