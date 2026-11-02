// Method B reads what a declaration writes with TypeScript's parser; it never asks the checker for a type, so it needs
// no program and is tested on source text.
import { describe, expect, it } from 'vitest'
import { prim, type Norm } from '../src/oracle-compare.ts'
import { annotationText, declaredType, declaredTypeOfAll } from '../src/reference-declared.ts'

const lit = (value: string): Norm => ({ k: 'lit', value })

describe('annotationText', () => {
  it('reads a property signature and keeps the annotation exactly as written', () => {
    expect.hasAssertions()
    expect(annotationText("size?: 'small' | 'large';")).toStrictEqual({ text: "'small' | 'large'" })
    expect(annotationText('onClick?: React.MouseEventHandler<HTMLElement>')).toStrictEqual({
      text: 'React.MouseEventHandler<HTMLElement>',
    })
  })

  it('reads a method signature as the function type it declares', () => {
    expect.hasAssertions()
    expect(annotationText('onChange(value: string, event?: Event): void;')).toStrictEqual({
      text: '(value: string, event?: Event) => void',
    })
    expect(annotationText('close();')).toStrictEqual({ text: '() => void' })
  })

  it('reads a class property and ignores its initializer', () => {
    expect.hasAssertions()
    expect(annotationText("label: string = 'x'")).toStrictEqual({ text: 'string' })
  })

  it('abstains when nothing is annotated or the text is not a member', () => {
    expect.hasAssertions()
    expect(annotationText('count = 1')).toStrictEqual({ abstained: 'no annotation' })
    expect(annotationText('(((')).toStrictEqual({ abstained: 'unsupported declaration' })
    expect(annotationText('a: string; b: number')).toStrictEqual({ abstained: 'unsupported declaration' })
  })
})

describe('declaredType', () => {
  it('reduces the written type to the shape the checker side is compared in', () => {
    expect.hasAssertions()
    expect(declaredType('count: number;')).toStrictEqual({ text: 'number', norm: prim('number') })
    expect(declaredType("size?: 'small' | 'large'")).toStrictEqual({
      text: "'small' | 'large'",
      norm: { k: 'union', members: [lit('small'), lit('large')] },
    })
  })

  it('names a type it does not expand, and abstains on one the declaration leaves to be computed', () => {
    expect.hasAssertions()
    expect(declaredType('mode: Mode')).toStrictEqual({ text: 'Mode', norm: { k: 'name', name: 'Mode' } })
    expect(declaredType("value: Props['value']")).toStrictEqual({ text: "Props['value']", abstained: 'unreadable' })
    expect(declaredType('keys: keyof Props')).toStrictEqual({ text: 'keyof Props', abstained: 'unreadable' })
  })

  it('has no type for a prop with no declaration', () => {
    expect.hasAssertions()
    expect(declaredType(undefined)).toStrictEqual({ abstained: 'no declaration' })
  })
})

describe('declaredTypeOfAll', () => {
  it('answers when every declaration writes the same type, as merged interfaces can', () => {
    expect.hasAssertions()
    expect(declaredTypeOfAll(['a: string;', 'a: string'])).toStrictEqual({ text: 'string', norm: prim('string') })
  })

  it('abstains when the declarations differ, because the checker combines them in a way B cannot', () => {
    expect.hasAssertions()
    expect(declaredTypeOfAll(['a: string;', 'a: number;'])).toStrictEqual({ abstained: 'multiple declarations' })
  })

  it('abstains with no declaration at all', () => {
    expect.hasAssertions()
    expect(declaredTypeOfAll([])).toStrictEqual({ abstained: 'no declaration' })
  })
})
