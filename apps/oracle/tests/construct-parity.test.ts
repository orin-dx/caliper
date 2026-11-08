// The tsrs-against-Go parity run (construct-suite/README.md): every construct-suite row captured with the Go checker and
// with tsrs must agree. The comparison is tested with made-up captures; the live run skips, with a warning, when tsrs is
// not installed, and fails instead when REQUIRE_TSRS=1.
import { describe, expect, it } from 'vitest'
import {
  compareCaptures,
  failed,
  lineDiff,
  loadTsrs,
  renderReport,
  runParity,
  type Capture,
  type RowFacts,
} from '../src/construct-parity.ts'
import { prim } from '../src/oracle-compare.ts'
import { lit } from './lit.ts'

const facts = (over: Partial<RowFacts> = {}): RowFacts => ({
  norm: prim('string'),
  text: 'string',
  flags: 4,
  intrinsicName: 'string',
  ...over,
})
const capture = (rows: Record<string, RowFacts>): Capture => rows

describe('the parity comparison', () => {
  it('counts rows that agree in every fact as identical', () => {
    expect.hasAssertions()
    const rows = capture({ 'a/01-x': facts(), 'a/02-y': facts({ text: 'number', norm: prim('number') }) })
    const result = compareCaptures(rows, structuredClone(rows), {})
    expect(result).toMatchObject({ rows: 2, identical: 2, unexpected: [], staleAllowed: [], missing: [] })
    expect(failed(result)).toBe(false)
  })

  it('reports a different normalized type with both texts and a diff', () => {
    expect.hasAssertions()
    const go = capture({ 'a/01-x': facts({ norm: lit('"a"', 'string'), text: '"a"' }) })
    const tsrs = capture({ 'a/01-x': facts({ norm: lit('"b"', 'string'), text: '"b"' }) })
    const result = compareCaptures(go, tsrs, {})
    expect(result.unexpected.map((d) => [d.id, d.facts])).toStrictEqual([['a/01-x', ['norm', 'text']]])
    expect(failed(result)).toBe(true)
    const report = renderReport(result)
    expect(report).toContain('UNEXPECTED a/01-x')
    expect(report).toContain('go:   "a"')
    expect(report).toContain('tsrs: "b"')
    expect(report).toMatch(/^ {4}- .*a/m)
    expect(report).toMatch(/^ {4}\+ .*b/m)
  })

  it('catches the error type reading as any: the flags and intrinsicName differ, the text does not', () => {
    expect.hasAssertions()
    const go = capture({ 'a/01-x': facts({ norm: { k: 'error' }, text: 'any', flags: 1, intrinsicName: 'error' }) })
    const tsrs = capture({ 'a/01-x': facts({ norm: { k: 'error' }, text: 'any', flags: 1, intrinsicName: 'any' }) })
    const result = compareCaptures(go, tsrs, {})
    expect(result.unexpected.map((d) => d.facts)).toStrictEqual([['intrinsicName']])
  })

  it('lets an allow-listed difference pass and says why', () => {
    expect.hasAssertions()
    const go = capture({ 'a/01-x': facts({ text: 'import("/a/lib").T' }) })
    const tsrs = capture({ 'a/01-x': facts({ text: 'import("/b/lib").T' }) })
    const result = compareCaptures(go, tsrs, { 'a/01-x': 'lib file path' })
    expect(result.unexpected).toStrictEqual([])
    expect(result.allowed.map((a) => a.reason)).toStrictEqual(['lib file path'])
    expect(failed(result)).toBe(false)
  })

  it('fails on an allow-listed row that no longer differs, or is not a row', () => {
    expect.hasAssertions()
    const rows = capture({ 'a/01-x': facts() })
    const result = compareCaptures(rows, structuredClone(rows), { 'a/01-x': 'was a path', 'gone/01-y': 'removed' })
    expect(result.staleAllowed.map((s) => s.id)).toStrictEqual(['a/01-x', 'gone/01-y'])
    expect(failed(result)).toBe(true)
    expect(renderReport(result)).toContain('STALE ALLOW-LIST a/01-x')
  })

  it('fails on a row only one side captured', () => {
    expect.hasAssertions()
    const result = compareCaptures(capture({ 'a/01-x': facts() }), capture({}), {})
    expect(result.missing).toStrictEqual(['a/01-x: only the Go checker captured it'])
    expect(failed(result)).toBe(true)
  })

  it('ignores the symbol id of a unique-symbol key', () => {
    expect.hasAssertions()
    const go = capture({ 'a/01-x': facts({ text: '{ __@tag@123: string }' }) })
    const tsrs = capture({ 'a/01-x': facts({ text: '{ __@tag@987: string }' }) })
    expect(compareCaptures(go, tsrs, {}).identical).toBe(1)
  })

  it('diffs two texts line by line', () => {
    expect.hasAssertions()
    expect(lineDiff('a\nb\nc', 'a\nx\nc')).toBe('  a\n- b\n+ x\n  c')
  })
})

const tsrs = await loadTsrs()
if (tsrs === undefined) {
  console.warn(
    'construct parity skipped: the tsrs package is not installed (pnpm install). Set REQUIRE_TSRS=1 to fail instead.',
  )
}

describe('the live parity run', () => {
  it.runIf(process.env.REQUIRE_TSRS === '1')('has tsrs installed when REQUIRE_TSRS=1', () => {
    expect(tsrs, 'REQUIRE_TSRS=1 but tsrs is not installed (pnpm install)').toBeDefined()
  })

  it.skipIf(tsrs === undefined)('finds every construct-suite row identical, bar the allow-list', async () => {
    expect.hasAssertions()
    const result = await runParity(tsrs!)
    expect(failed(result) ? renderReport(result) : 'no differences').toBe('no differences')
    expect(result.rows).toBeGreaterThan(0)
  })
})
