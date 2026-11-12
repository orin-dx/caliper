import { describe, expect, it } from 'vitest'
import { assertSchemaVersion, SUPPORTED_SCHEMA_VERSION } from '../src/schema-version.ts'

describe('assertSchemaVersion', () => {
  it('returns output of the supported version', () => {
    const output = { schemaVersion: SUPPORTED_SCHEMA_VERSION, components: {} }
    expect(assertSchemaVersion(output, 'caliper JSON')).toBe(output)
  })

  it('rejects another version, naming both', () => {
    expect(() => assertSchemaVersion({ schemaVersion: 99 }, 'caliper JSON')).toThrow(
      `The caliper JSON has schema version 99, and the oracle reads version ${SUPPORTED_SCHEMA_VERSION}. ` +
        'Run it with a caliper-docgen binary of the same release.',
    )
  })

  it('treats output without a version as version 1', () => {
    expect(() => assertSchemaVersion({}, 'caliper JSON')).toThrow('has schema version 1,')
  })
})
