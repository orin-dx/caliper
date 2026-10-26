import { existsSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { experimental_docgenProvider, isDocgenProviderEnabled } from '../../src/preset.js'

describe('preset', () => {
  it('registers the provider after existing ones, with only cloneable options', async () => {
    const react = { moduleSpecifier: '/react/docgen-worker.js' }
    const options = { srcDirs: ['lib'], fallback: 'never' as const, presets: { apply: () => {} } }
    const [first, ours, ...rest] = await experimental_docgenProvider([react], options)
    expect(first).toBe(react)
    expect(rest).toEqual([])
    expect(ours?.options).toEqual({
      srcDirs: ['lib'],
      exclude: undefined,
      tsconfigPath: undefined,
      skipHtmlProps: undefined,
      htmlAttributes: undefined,
      cache: undefined,
      loadDependencies: undefined,
      collapseInheritedProps: undefined,
      lazy: undefined,
      warmUp: undefined,
      fillAfter: undefined,
      fallback: 'never',
    })
    expect(ours?.moduleSpecifier).toMatch(/provider\.(js|ts)$/)
    expect(existsSync(ours!.moduleSpecifier.replace(/\.js$/, '.ts')) || existsSync(ours!.moduleSpecifier)).toBe(true)
  })

  it('turns on the docgen server', async () => {
    expect(await isDocgenProviderEnabled()).toBe(true)
  })
})
