// The control sample is seeded and stratified: the same on every run, and spread over the kinds of prop, not over the
// kinds there happen to be most of.
import { describe, expect, it } from 'vitest'
import { keyOf, seeded, shuffled, stratified, stratumOf, type Candidate } from '../src/reference-sample.ts'

function candidate(prop: string, over: Partial<Candidate> = {}): Candidate {
  return {
    corpus: 'c',
    file: 'a/A.tsx',
    component: 'A',
    prop,
    origin: 'own',
    shape: 'plain',
    kind: 'other',
    ...over,
  }
}

describe('stratumOf', () => {
  it('puts a prop in the first stratum it fits, the harder shapes first', () => {
    expect.hasAssertions()
    expect(stratumOf(candidate('p'), true)).toBe('listed')
    expect(stratumOf(candidate('p', { kind: 'generic', shape: 'mapped' }), false)).toBe('generic')
    expect(stratumOf(candidate('p', { shape: 'mapped', kind: 'function' }), false)).toBe('mapped')
    expect(stratumOf(candidate('p', { shape: 'intersection', origin: 'html' }), false)).toBe('intersection')
    expect(stratumOf(candidate('p', { kind: 'function', origin: 'dependency' }), false)).toBe('function')
    expect(stratumOf(candidate('p', { kind: 'union_literal' }), false)).toBe('union_literal')
    expect(stratumOf(candidate('p', { origin: 'dependency' }), false)).toBe('dependency')
    expect(stratumOf(candidate('p', { origin: 'html' }), false)).toBe('html')
    expect(stratumOf(candidate('p', { origin: 'extends' }), false)).toBe('extends')
    expect(stratumOf(candidate('p'), false)).toBe('own')
  })
})

describe('seeded', () => {
  it('gives the same sequence for the same seed and another for another', () => {
    expect.hasAssertions()
    const first = Array.from({ length: 3 }, seeded('caliper'))
    const again = Array.from({ length: 3 }, seeded('caliper'))
    const other = Array.from({ length: 3 }, seeded('other'))
    expect(first).toStrictEqual(again)
    expect(first).not.toStrictEqual(other)
    expect(first.every((n) => n >= 0 && n < 1)).toBe(true)
  })

  it('shuffles without losing or repeating an item', () => {
    expect.hasAssertions()
    const items = Array.from({ length: 20 }, (_, i) => i)
    const out = shuffled(items, seeded('s'))
    expect(out.toSorted((a, b) => a - b)).toStrictEqual(items)
    expect(out).not.toStrictEqual(items)
  })
})

describe('stratified', () => {
  // 60 own props, 6 html, 3 generic: a plain sample of 12 would be all own.
  const pool = [
    ...Array.from({ length: 60 }, (_, i) => candidate(`own${i}`)),
    ...Array.from({ length: 6 }, (_, i) => candidate(`html${i}`, { origin: 'html' })),
    ...Array.from({ length: 3 }, (_, i) => candidate(`gen${i}`, { kind: 'generic' })),
  ]
  const count = (picked: Candidate[], stratum: string) => picked.filter((c) => stratumOf(c, false) === stratum).length

  it('takes the small strata whole and shares the rest out', () => {
    expect.hasAssertions()
    const picked = stratified(pool, 12, 'seed')
    expect(picked).toHaveLength(12)
    expect(count(picked, 'generic')).toBe(3)
    expect(count(picked, 'html')).toBe(5)
    expect(count(picked, 'own')).toBe(4)
  })

  it('does not depend on the order the checker visited the files in', () => {
    expect.hasAssertions()
    const a = stratified(pool, 12, 'seed').map(keyOf)
    const b = stratified(pool.toReversed(), 12, 'seed').map(keyOf)
    expect(a).toStrictEqual(b)
  })

  it('draws another sample for another seed, and never more than the pool has', () => {
    expect.hasAssertions()
    expect(stratified(pool, 12, 'one').map(keyOf)).not.toStrictEqual(stratified(pool, 12, 'two').map(keyOf))
    expect(stratified(pool, 500, 'seed')).toHaveLength(pool.length)
  })
})
