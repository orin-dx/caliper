// Where a declaration lives is one rule for the oracle and the weekly: both read src/path-classes.json and both run the
// cases of tests/fixtures/path-class-cases.json (scripts/test_path_classes.py is the other half).
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { isDependency, isLibFile, isReactTypes } from '../src/path-class.ts'

interface Case {
  why: string
  path: string
  dependency: boolean
  reactTypes: boolean
  libFile: boolean
}

const cases: Case[] = JSON.parse(readFileSync(join(import.meta.dirname, 'fixtures', 'path-class-cases.json'), 'utf8'))

describe('path classes', () => {
  it.each(cases)('$why ($path)', ({ path, dependency, reactTypes, libFile }) => {
    expect({ dependency: isDependency(path), reactTypes: isReactTypes(path), libFile: isLibFile(path) }).toStrictEqual({
      dependency,
      reactTypes,
      libFile,
    })
  })

  it('the cases hold every combination the rules can give', () => {
    expect.hasAssertions()
    const seen = new Set(cases.map((c) => `${c.dependency}/${c.reactTypes}/${c.libFile}`))

    expect(seen).toStrictEqual(
      new Set(['false/false/false', 'true/false/false', 'true/true/false', 'true/false/true', 'false/false/true']),
    )
  })
})
