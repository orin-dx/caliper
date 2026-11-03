import { existsSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import type { CaliperStorybookOptions } from '../../src/index.js'
import { OPTION_KEYS } from '../../src/option-keys.js'
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

  it('forwards every option key to the worker', async () => {
    const options = {
      srcDirs: ['lib'],
      exclude: ['x'],
      tsconfigPath: 'tsconfig.json',
      skipHtmlProps: true,
      htmlAttributes: 'curated',
      cache: false,
      loadDependencies: false,
      collapseInheritedProps: 'all',
      lazy: false,
      warmUp: true,
      fillAfter: 5,
      semanticTypes: 'on-demand',
      tscPath: '/bin/tsc',
      semanticWaitMs: 7,
      fallback: 'never',
    } satisfies Record<keyof CaliperStorybookOptions, unknown>
    const [ours] = await experimental_docgenProvider([], options)
    expect(Object.keys(ours!.options!).sort()).toEqual([...OPTION_KEYS].sort())
    expect(ours?.options).toEqual(options)
  })

  it('turns on the docgen server', async () => {
    expect(await isDocgenProviderEnabled()).toBe(true)
  })
})
