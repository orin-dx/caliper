import { existsSync } from 'node:fs'
import { logger } from 'storybook/internal/node-logger'
import type { IndexEntry, StoryIndex } from 'storybook/internal/types'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { CaliperStorybookOptions } from '../../src/index.js'
import { experimental_devServer, experimental_docgenProvider, isDocgenProviderEnabled } from '../../src/preset.js'

/** Storybook's docgen service as the preset finds it: every `getService` call, and the extractions asked of it. */
const core = vi.hoisted(() => ({
  services: [] as string[],
  extracted: [] as string[],
  missing: false,
}))

vi.mock('storybook/internal/core-server', () => ({
  getService: (id: string, options: { internal?: boolean }) => {
    core.services.push(`${id} internal=${String(options.internal)}`)
    if (core.missing) throw new Error(`no service ${id}`)
    return {
      commands: {
        extractDocgen: async (input: { id: string }) => {
          core.extracted.push(input.id)
          return { id: input.id }
        },
      },
    }
  },
}))

function story(id: string): IndexEntry {
  const title = id.split('--')[0]!
  return { type: 'story', subtype: 'story', id, name: id, title, importPath: `./src/${title}.stories.tsx` }
}

const stories: StoryIndex = {
  v: 5,
  entries: Object.fromEntries(
    ['button--primary', 'button--ghost', 'card--basic', 'menu--basic'].map((id) => [id, story(id)]),
  ),
}

function devServerOptions(extra: CaliperStorybookOptions = {}) {
  const asked: string[] = []
  return {
    asked,
    options: {
      ...extra,
      presets: {
        apply: async (extension: 'storyIndexGenerator') => {
          asked.push(extension)
          return { getIndex: async () => stories }
        },
      },
    },
  }
}

afterEach(() => {
  core.services.length = 0
  core.extracted.length = 0
  core.missing = false
  vi.restoreAllMocks()
})

describe('prefetch at dev server start', () => {
  it("documents the opening story's component through Storybook's docgen service by default", async () => {
    const app = { name: 'app' }
    const { options, asked } = devServerOptions()
    expect(await experimental_devServer(app, options)).toBe(app)
    await vi.waitFor(() => expect(core.extracted).toEqual(['button']))
    expect(core.services).toEqual(['core/docgen internal=true'])
    expect(asked).toEqual(['storyIndexGenerator'])
  })

  it('documents the first `prefetchStories` components in index order', async () => {
    await experimental_devServer({}, devServerOptions({ prefetchStories: 2 }).options)
    await vi.waitFor(() => expect(core.extracted).toEqual(['button', 'card']))
  })

  it('asks nothing with `prefetchStories: 0`', async () => {
    const { options, asked } = devServerOptions({ prefetchStories: 0 })
    await experimental_devServer({}, options)
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(core.services).toEqual([])
    expect(asked).toEqual([])
  })

  it('logs and returns the app when Storybook has no docgen service', async () => {
    core.missing = true
    const debug = vi.spyOn(logger, 'debug').mockImplementation(() => {})
    const app = { name: 'app' }
    expect(await experimental_devServer(app, devServerOptions().options)).toBe(app)
    await vi.waitFor(() =>
      expect(debug).toHaveBeenCalledWith('[caliper-docgen] prefetch stopped: no service core/docgen'),
    )
    expect(core.extracted).toEqual([])
  })
})

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

  it('fails when Storybook starts for an option that is not a choice', () => {
    expect(() => experimental_docgenProvider([], { fallback: 'always' as never })).toThrow(
      "[caliper-docgen] fallback is 'always', expected 'degraded' or 'never'",
    )
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
      prefetchStories: 3,
      semanticTypes: 'on-demand',
      tscPath: '/bin/tsc',
      semanticWaitMs: 7,
      semanticWaitEmptyMs: 70,
      fallback: 'never',
    } satisfies Record<keyof CaliperStorybookOptions, unknown>
    const [ours] = await experimental_docgenProvider([], options)
    expect(ours?.options).toEqual(options)
  })

  it("leaves out Storybook's own cache, which shares the options object with the addon's", async () => {
    const storybookCache = { get: async () => undefined, set: async () => undefined }
    const [ours] = await experimental_docgenProvider([], { srcDirs: ['lib'], cache: storybookCache })
    expect(ours?.options).toHaveProperty('cache', undefined)
    expect(structuredClone(ours?.options)).toEqual({ srcDirs: ['lib'] })
  })

  it('turns on the docgen server', async () => {
    expect(await isDocgenProviderEnabled()).toBe(true)
  })
})
