// How the checker's type (A) and the declared type (B) become one reference, and how a tool's type is scored against it.
import { describe, expect, it } from 'vitest'
import { prim, type Norm } from '../src/oracle-compare.ts'
import { reference, score, type Reference } from '../src/reference-judge.ts'
import { lit } from './lit.ts'

const union = (...members: Norm[]): Norm => ({ k: 'union', members })
const name = (value: string, alias?: string): Norm => ({ k: 'name', name: value, alias })
const sizes = union(lit('small'), lit('large'))

describe('reference', () => {
  it('is agreed on both methods when the declaration and the checker say the same type', () => {
    expect.hasAssertions()
    expect(reference(prim('number'), prim('number'))).toStrictEqual({
      kind: 'agreed',
      basis: 'both',
      norm: prim('number'),
    })
  })

  it('counts a name for the alias the checker expanded as agreement', () => {
    expect.hasAssertions()
    const expanded: Norm = { ...sizes, alias: 'Size' }
    expect(reference(expanded, name('Size'))).toStrictEqual({ kind: 'agreed', basis: 'both', norm: expanded })
  })

  it('stands on the checker alone when the declaration abstained', () => {
    expect.hasAssertions()
    expect(reference(prim('string'), undefined)).toStrictEqual({
      kind: 'agreed',
      basis: 'checker',
      norm: prim('string'),
      why: 'abstained',
    })
  })

  it('stands on the checker alone when the declaration is looser than the checker, as a name the checker evaluated', () => {
    expect.hasAssertions()
    expect(reference(sizes, name('Unknown'))).toStrictEqual({
      kind: 'agreed',
      basis: 'checker',
      norm: sizes,
      why: 'looser',
    })
  })

  it('stands on the declaration alone where the checker has no type', () => {
    expect.hasAssertions()
    expect(reference({ k: 'error' }, prim('number'))).toStrictEqual({
      kind: 'agreed',
      basis: 'declaration',
      norm: prim('number'),
      why: 'abstained',
    })
    expect(reference(prim('any'), prim('number'))).toStrictEqual({
      kind: 'agreed',
      basis: 'declaration',
      norm: prim('number'),
      why: 'abstained',
    })
  })

  it('has no reference when neither method has a type', () => {
    expect.hasAssertions()
    expect(reference(prim('any'), undefined)).toStrictEqual({ kind: 'none' })
    expect(reference({ k: 'error' }, undefined)).toStrictEqual({ kind: 'none' })
  })

  it('is contested when both methods have a type and the types differ', () => {
    expect.hasAssertions()
    expect(reference(prim('string'), prim('number'))).toStrictEqual({
      kind: 'contested',
      checker: prim('string'),
      declared: prim('number'),
    })
  })
})

describe('a reference the declaration stands behind alone', () => {
  // The checker could not resolve `Mode` (an unresolved import), so the declaration's name is the whole reference.
  const mode = reference({ k: 'error' }, name('Mode'))

  it('scores a tool that gives the same name as correct, whatever it is called by namespace', () => {
    expect.hasAssertions()
    expect(score(name('Mode'), mode, false)).toStrictEqual({ outcome: 'correct', basis: 'declaration' })
    expect(score(name('Mode'), reference({ k: 'error' }, name('Ns.Mode')), false)).toStrictEqual({
      outcome: 'correct',
      basis: 'declaration',
    })
  })

  it('cannot call another type wrong, because it cannot see what the name stands for', () => {
    expect.hasAssertions()
    expect(score(prim('string'), mode, false)).toStrictEqual({ outcome: 'less precise', basis: 'declaration' })
    expect(score(name('Other'), mode, false)).toStrictEqual({ outcome: 'less precise', basis: 'declaration' })
  })

  it('finds a name inside a written type too', () => {
    expect.hasAssertions()
    const modes = reference({ k: 'error' }, { k: 'array', element: name('Mode') })
    expect(score({ k: 'array', element: name('Mode') }, modes, false)).toStrictEqual({
      outcome: 'correct',
      basis: 'declaration',
    })
  })
})

describe('score', () => {
  const agreed: Reference = { kind: 'agreed', basis: 'both', norm: sizes }

  it('calls a type correct when it matches an agreed reference, and keeps what stood behind it', () => {
    expect.hasAssertions()
    expect(score(sizes, agreed, false)).toStrictEqual({ outcome: 'correct', basis: 'both' })
    expect(score(sizes, { ...agreed, basis: 'checker' }, false)).toStrictEqual({ outcome: 'correct', basis: 'checker' })
  })

  it('calls a looser type less precise and a contradicting one wrong', () => {
    expect.hasAssertions()
    expect(score(prim('string'), agreed, false)).toStrictEqual({ outcome: 'less precise', basis: 'both' })
    expect(score({ k: 'array', element: prim('string') }, agreed, false)).toStrictEqual({
      outcome: 'wrong',
      basis: 'both',
    })
  })

  it('credits a prop a collapsed layer names as surfaced, and a prop nothing names as missing', () => {
    expect.hasAssertions()
    expect(score(undefined, agreed, true)).toStrictEqual({ outcome: 'layer' })
    expect(score(undefined, agreed, false)).toStrictEqual({ outcome: 'missing' })
  })

  it('has nothing to score against when there is no reference', () => {
    expect.hasAssertions()
    expect(score(prim('string'), { kind: 'none' }, false)).toStrictEqual({ outcome: 'no reference' })
  })

  it('does not judge a contested prop, but records which method the tool sided with', () => {
    expect.hasAssertions()
    const contested: Reference = { kind: 'contested', checker: prim('string'), declared: prim('number') }
    expect(score(prim('string'), contested, false)).toStrictEqual({ outcome: 'contested', leans: 'checker' })
    expect(score(prim('number'), contested, false)).toStrictEqual({ outcome: 'contested', leans: 'declaration' })
    expect(score(prim('boolean'), contested, false)).toStrictEqual({ outcome: 'contested', leans: undefined })
  })
})
