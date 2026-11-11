// A component of the checker is paired with Caliper's by file and export name (ADR 0027), never by display name.
import { realpathSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { exportedAs, pathOfExport, exportPathKey, type CaliperComponent } from '../src/component-join.ts'
import { rowKeysOf } from '../src/construct-suite.ts'

const exports = (...paths: string[]): CaliperComponent['exports'] =>
  paths.map((path) => {
    const [name, ...members] = path.split('.')
    return { name, members }
  })

/** A file's components the way the oracle holds them: Caliper's output key and the component. */
const file = (...components: [key: string, displayName: string, exports: CaliperComponent['exports']][]) =>
  components.map(([key, displayName, exported]) => ({ key, component: { displayName, exports: exported } }))

describe('exportedAs', () => {
  it('keeps _Card and Card, two components of one file, apart', () => {
    const candidates = file(['_Card', '_Card', exports('_Card')], ['Card', 'Card', exports('Card')])
    expect(exportedAs(candidates, ['Card']).map((c) => c.key)).toStrictEqual(['Card'])
    expect(exportedAs(candidates, ['_Card']).map((c) => c.key)).toStrictEqual(['_Card'])
  })

  it('finds a component exported under another name than its own, and not the unexported one that has that name', () => {
    const candidates = file(['_Card', '_Card', exports('Card')], ['Card', 'Card', exports()])
    expect(exportedAs(candidates, ['Card']).map((c) => c.key)).toStrictEqual(['_Card'])
  })

  it('finds a default export and a named export of one function as one component', () => {
    const candidates = file(['Foo', 'Foo', exports('default', 'Foo')])
    expect(exportedAs(candidates, ['default', 'Foo']).map((c) => c.key)).toStrictEqual(['Foo'])
  })

  it('does not take a sub-component attached to an export for the export, nor the export for the sub-component', () => {
    const candidates = file(['Menu', 'Menu', exports('default')], ['Item', 'Item', exports('default.Item')])
    expect(exportedAs(candidates, ['default']).map((c) => c.key)).toStrictEqual(['Menu'])
    expect(exportedAs(candidates, ['Item'])).toStrictEqual([])
  })

  it('finds no component for a name the module does not export', () => {
    expect(exportedAs(file(['Card', 'Card', exports('Card')]), ['Other'])).toStrictEqual([])
    expect(exportedAs(file(['Card', 'Card', exports()]), ['Card'])).toStrictEqual([])
  })
})

describe('the row of a component', () => {
  const here = realpathSync(import.meta.filename)
  const at = (...paths: string[]) => rowKeysOf({ filePath: here, exports: exports(...paths) })

  it('is its file and the way its module exports it', () => {
    expect(at('default')).toStrictEqual([exportPathKey(here, ['default'])])
    expect(at('default.Item', 'Item')).toStrictEqual([
      exportPathKey(here, ['default', 'Item']),
      exportPathKey(here, ['Item']),
    ])
  })

  it('tells A.Item from B.Item, which share a display name', () => {
    expect(at('A.Item')).not.toStrictEqual(at('B.Item'))
  })

  it('has an export path that starts at the export name', () => {
    expect(pathOfExport({ name: 'default', members: ['Item'] })).toStrictEqual(['default', 'Item'])
  })
})
