// src/vocabulary.ts is the one definition of the oracle's verdict names; src/vocabulary.json is what the Python scripts
// read. A list edited in one and not written to the other fails here.
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { compare, prim } from '../src/oracle-compare.ts'
import { AGREE, ROW_VERDICTS, SEVERITY, VERDICTS, vocabulary } from '../src/vocabulary.ts'

const written = JSON.parse(readFileSync(join(import.meta.dirname, '..', 'src', 'vocabulary.json'), 'utf8'))

describe('the verdict vocabulary', () => {
  it('src/vocabulary.json is src/vocabulary.ts written as data', () => {
    expect.hasAssertions()
    expect(written, 'run `pnpm --filter caliper-docgen-oracle vocabulary`').toStrictEqual(vocabulary)
  })

  it('the lists are made of the names they derive from', () => {
    expect.hasAssertions()
    expect(AGREE.every((v) => VERDICTS.includes(v))).toBe(true)
    expect(SEVERITY.every((v) => VERDICTS.includes(v))).toBe(true)
    expect(VERDICTS.filter((v) => !ROW_VERDICTS.includes(v))).toStrictEqual([])
    expect(ROW_VERDICTS.filter((v) => !VERDICTS.some((verdict) => verdict === v))).toStrictEqual([
      'opaque with marker',
      'missing',
    ])
  })

  it('every verdict the comparison can give is in the list', () => {
    expect.hasAssertions()
    const opaque = { k: 'opaque' } as const
    const given = new Set([
      compare(prim('string'), prim('string')),
      compare(prim('string'), { k: 'lit', value: 'a', of: 'string' }),
      compare(prim('string'), prim('number')),
      compare(prim('string'), prim('any')),
      compare(prim('string'), opaque),
    ])

    expect(given).toStrictEqual(new Set(['identical', 'less precise', 'wrong', 'checker any/error', 'not judged']))
    expect([...given].every((v) => VERDICTS.includes(v))).toBe(true)
  })
})
