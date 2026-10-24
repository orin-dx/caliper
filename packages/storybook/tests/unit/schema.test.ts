import { describe, expect, it } from 'vitest'
import { DocgenSession } from '../../src/session.js'
import { SUPPORTED_SCHEMA_VERSION, assertSchemaVersion, expandDiagnostics } from '../../src/schema.js'
import { FakeBinding } from './fake-binding.js'

describe('assertSchemaVersion', () => {
  it('returns output of the version it reads', () => {
    const versioned = { schemaVersion: SUPPORTED_SCHEMA_VERSION }
    expect(assertSchemaVersion(versioned, 'session index')).toBe(versioned)
  })

  it('counts output with no version as version 1, which this package no longer reads', () => {
    expect(() => assertSchemaVersion({}, 'session index')).toThrow(
      `has schema version 1, and this package reads version ${SUPPORTED_SCHEMA_VERSION}.`,
    )
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

describe('expandDiagnostics', () => {
  const diagnostic = {
    severity: 'warning',
    message: 'm',
    file: null,
    line: null,
    column: null,
    help: null,
    code: 'x',
  } as const

  it('replaces an affects list id with the names it stands for and leaves a diagnostic with none alone', () => {
    const expanded = expandDiagnostics([{ ...diagnostic, affects: 'abc' }, diagnostic], { abc: ['Button', 'Chip'] })
    expect(expanded).toEqual([{ ...diagnostic, affects: ['Button', 'Chip'] }, diagnostic])
  })

  it('names the id the name lists lack', () => {
    expect(() => expandDiagnostics([{ ...diagnostic, affects: 'abc' }], {})).toThrow(
      'The caliper-docgen output refers to name list abc, which its nameLists lacks.',
    )
  })
})
