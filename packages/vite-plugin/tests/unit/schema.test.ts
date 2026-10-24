import { describe, expect, it } from 'vitest'
import { SUPPORTED_SCHEMA_VERSION, assertSchemaVersion } from '../../src/schema.js'

describe('assertSchemaVersion', () => {
  it('returns output of the version it reads, and output with no version, which has that shape', () => {
    const versioned = { schemaVersion: SUPPORTED_SCHEMA_VERSION }
    const unversioned = {}
    expect(assertSchemaVersion(versioned, 'output')).toBe(versioned)
    expect(assertSchemaVersion(unversioned, 'output')).toBe(unversioned)
  })

  it('names the version, the one it reads and what to update when the version is different', () => {
    expect(() => assertSchemaVersion({ schemaVersion: SUPPORTED_SCHEMA_VERSION + 1 }, 'update')).toThrow(
      `The caliper-docgen update has schema version ${SUPPORTED_SCHEMA_VERSION + 1}, and this package reads version ${SUPPORTED_SCHEMA_VERSION}. ` +
        'Update caliper-docgen-vite-plugin and caliper-docgen-napi to the same release.',
    )
  })
})
