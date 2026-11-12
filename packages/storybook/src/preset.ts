import { fileURLToPath } from 'node:url'
import { logger } from 'storybook/internal/node-logger'
import type { DocgenProviderDescriptor, StoryIndex } from 'storybook/internal/types'
import type { CaliperStorybookOptions } from './index.js'
import { assertValidOptions } from './validate.js'
import { OPTION_KEYS } from './option-keys.js'
import { prefetch } from './prefetch.js'

/**
 * What a preset function receives: Storybook's own options merged with the addon's. Storybook's `cache` is its
 * file-system cache object, not the addon's boolean.
 */
type PresetOptions = Omit<CaliperStorybookOptions, 'cache'> & { cache?: unknown }

/** The part of Storybook's options the prefetch reads: its presets, for the story index. */
interface DevServerOptions extends PresetOptions {
  presets: { apply(extension: 'storyIndexGenerator'): Promise<{ getIndex(): Promise<StoryIndex> } | undefined> }
}

/** The addon's options travel to the docgen worker by structured clone, so only these plain fields go. */
function workerOptions(options: PresetOptions): CaliperStorybookOptions {
  const cache = typeof options.cache === 'boolean' ? options.cache : undefined
  return Object.fromEntries(OPTION_KEYS.map((key) => [key, key === 'cache' ? cache : options[key]]))
}

/** Registered last, the provider wraps React's: it answers first and hands on what it can't document. */
export const experimental_docgenProvider = (
  existing: DocgenProviderDescriptor[] = [],
  options: PresetOptions = {},
): DocgenProviderDescriptor[] => {
  assertValidOptions(options)
  return [
    ...existing,
    { moduleSpecifier: fileURLToPath(new URL('provider.js', import.meta.url)), options: workerOptions(options) },
  ]
}

/** Storybook runs docgen providers only with `features.docgenServer`, which this turns on unless `main.ts` sets it. */
export const isDocgenProviderEnabled = (): boolean => true

const PREFETCH_STORIES = 1

/**
 * When the dev server starts, before a browser has asked, documents the components of the first `prefetchStories`
 * stories through Storybook's docgen service: that starts its worker, the session and the checker, and stores the
 * opening story's answer where the browser reads it. Not awaited, and best effort: a failure costs the first story the
 * time it would have saved.
 */
export const experimental_devServer = <App>(app: App, options: DevServerOptions): App => {
  const count = options.prefetchStories ?? PREFETCH_STORIES
  if (count > 0) void startPrefetch(options, count)
  return app
}

async function startPrefetch(options: DevServerOptions, count: number): Promise<void> {
  try {
    const { getService } = await import('storybook/internal/core-server')
    // Registered by Storybook's own `experimental_devServer`, which runs before an addon's.
    const docgen = getService('core/docgen', { internal: true })
    await prefetch(
      {
        index: async () => {
          const generator = await options.presets.apply('storyIndexGenerator')
          if (!generator) throw new Error('Storybook has no story index')
          return generator.getIndex()
        },
        extract: (id) => docgen.commands.extractDocgen({ id }),
      },
      count,
    )
  } catch (error) {
    logger.debug(`[caliper-docgen] prefetch stopped: ${(error as Error).message}`)
  }
}
