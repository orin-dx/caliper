import { describe, expect, it } from 'vitest'
import { assertValidOptions } from '../../src/validate.js'

describe('assertValidOptions', () => {
  it('accepts every documented choice and leaves unset options alone', () => {
    expect(() => assertValidOptions({})).not.toThrow()
    for (const fallback of ['degraded', 'never'] as const) {
      expect(() => assertValidOptions({ fallback })).not.toThrow()
    }
    for (const semanticTypes of ['off', 'on-demand'] as const) {
      expect(() => assertValidOptions({ semanticTypes })).not.toThrow()
    }
  })

  it('names a fallback that is not a choice instead of treating it as never', () => {
    expect(() => assertValidOptions({ fallback: 'Degraded' as never })).toThrow(
      "[caliper-docgen] fallback is 'Degraded', expected 'degraded' or 'never'",
    )
  })

  it('names a semanticTypes that is not a choice', () => {
    expect(() => assertValidOptions({ semanticTypes: 'on_demand' as never })).toThrow(
      "[caliper-docgen] semanticTypes is 'on_demand', expected 'off' or 'on-demand'",
    )
  })
})
