import { describe, expect, it } from 'vitest'
import { assertValidOptions } from '../../src/validate.js'

describe('assertValidOptions', () => {
  it('accepts every documented choice and leaves unset options alone', () => {
    expect(() => assertValidOptions({})).not.toThrow()
    for (const fallback of ['degraded', 'never'] as const) {
      expect(() => assertValidOptions({ fallback })).not.toThrow()
    }
    for (const checker of [true, false]) {
      expect(() => assertValidOptions({ checker })).not.toThrow()
    }
  })

  it('names a fallback that is not a choice instead of treating it as never', () => {
    expect(() => assertValidOptions({ fallback: 'Degraded' as never })).toThrow(
      "[caliper-docgen] fallback is 'Degraded', expected 'degraded' or 'never'",
    )
  })

  it('names a checker that is not a boolean', () => {
    expect(() => assertValidOptions({ checker: 'on-demand' as never })).toThrow(
      "[caliper-docgen] checker is 'on-demand', expected true or false",
    )
  })

  it('refuses the old semanticTypes, which left unread would run the checker it meant to keep off', () => {
    expect(() => assertValidOptions({ semanticTypes: 'off' })).toThrow(
      "[caliper-docgen] semanticTypes was replaced by checker: the checker is on by default, and 'checker: false' turns it off",
    )
  })
})
