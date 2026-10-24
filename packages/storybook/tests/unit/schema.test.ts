import { describe, expect, it } from 'vitest'
import { DocgenSession } from '../../src/session.js'
import { SUPPORTED_SCHEMA_VERSION, assertSchemaVersion } from '../../src/schema.js'
import { FakeBinding } from './fake-binding.js'

describe('assertSchemaVersion', () => {
  it('returns output of the version it reads, and output with no version, which has that shape', () => {
    const versioned = { schemaVersion: SUPPORTED_SCHEMA_VERSION }
    const unversioned = {}
    expect(assertSchemaVersion(versioned, 'session index')).toBe(versioned)
    expect(assertSchemaVersion(unversioned, 'session index')).toBe(unversioned)
  })

  it('names the version, the one it reads and what to update when the version is different', () => {
    expect(() => assertSchemaVersion({ schemaVersion: SUPPORTED_SCHEMA_VERSION + 1 }, 'session index')).toThrow(
      `The caliper-docgen session index has schema version ${SUPPORTED_SCHEMA_VERSION + 1}, and this package reads version ${SUPPORTED_SCHEMA_VERSION}. ` +
        'Update caliper-docgen-storybook and caliper-docgen-napi to the same release.',
    )
  })

  it('stops a session from starting on an index of a different version', async () => {
    const binding = new FakeBinding({ components: {}, schemaVersion: SUPPORTED_SCHEMA_VERSION + 1 })
    const session = new DocgenSession({ srcDirs: ['/project/src'] }, binding)
    await expect(session.fresh()).rejects.toThrow('has schema version')
  })
})
