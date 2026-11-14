import { fileURLToPath } from 'node:url'
import { CURRENT_STORY_WAS_SET, SET_CURRENT_STORY } from 'storybook/internal/core-events'
import { logger } from 'storybook/internal/node-logger'
import type { DocgenProviderDescriptor, StoryIndex } from 'storybook/internal/types'
import type { CaliperStorybookOptions } from './index.js'
import { channel, componentOf, type Link, type ToDevServer, type ToProvider } from './live.js'
import { assertValidOptions } from './validate.js'
import { OPTION_KEYS } from './option-keys.js'
import { neighbours, prefetch } from './prefetch.js'

/** The part of Storybook's server channel the addon uses: the events the browser sends. */
interface ServerChannel {
  on(event: string, listener: (payload?: { storyId?: unknown }) => void): void
}

/**
 * What a preset function receives: Storybook's own options merged with the addon's. Storybook's `cache` is its
 * file-system cache object, not the addon's boolean.
 */
type PresetOptions = Omit<CaliperStorybookOptions, 'cache'> & { cache?: unknown }

/** The part of Storybook's options the prefetch reads: its presets, for the story index. */
export interface DevServerOptions extends PresetOptions {
  presets: { apply(extension: 'storyIndexGenerator'): Promise<{ getIndex(): Promise<StoryIndex> } | undefined> }
  /** Storybook's server channel, the same object for every hook of one dev server. */
  channel?: object
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

/** The channel events that say which story the browser shows: the manager's choice, then the preview's. */
const STORY_SHOWN = [SET_CURRENT_STORY, CURRENT_STORY_WAS_SET]

/** How the dev server asks Storybook's docgen service to document one component and store it where the browser reads it. */
export type Extract = (componentId: string) => Promise<unknown>

async function docgenService(): Promise<Extract> {
  const { getService } = await import('storybook/internal/core-server')
  // Registered by Storybook's own `experimental_devServer`, which runs before an addon's.
  const docgen = getService('core/docgen', { internal: true })
  return (id) => docgen.commands.extractDocgen({ id })
}

/**
 * The dev server's side of the addon. It documents the opening story before a browser asks, follows the story the
 * browser shows, has the provider resolve the next stories' components in the background, and has the docgen service
 * ask again for a component whose structural answer the checker has since replaced: the service stores the new answer
 * and the browser's Controls and docs, which read it live, show it without a reload. Only for a component some browser
 * shows; one the provider says was answered while no browser showed it is asked again, once, when one comes back to
 * it. The next stories' queue is one for the process: the last story opened in any browser sets it.
 */
export class DevServer {
  /**
   * The component each browser shows, by its channel's sender id. The channel says nothing when a tab closes, so a
   * closed tab's last component stays: at most one extra request when the checker answers it.
   */
  private readonly open = new Map<string, string>()
  /** Components the checker answered while no browser showed them, to ask again when one comes back. */
  private readonly answeredAway = new Set<string>()
  private readonly count: number
  private readonly options: DevServerOptions
  private readonly link: Link<ToProvider, ToDevServer>
  private readonly extractWith: () => Promise<Extract>

  constructor(
    options: DevServerOptions,
    link: Link<ToProvider, ToDevServer>,
    extractWith: () => Promise<Extract> = docgenService,
  ) {
    this.options = options
    this.link = link
    this.extractWith = extractWith
    this.count = options.prefetchStories ?? PREFETCH_STORIES
    link.listen((message) => {
      if (message.type !== 'answered') return
      if (this.onScreen(message.componentId)) void this.extract(message.componentId)
      else this.answeredAway.add(message.componentId)
    })
  }

  private async index(): Promise<StoryIndex> {
    const generator = await this.options.presets.apply('storyIndexGenerator')
    if (!generator) throw new Error('Storybook has no story index')
    return generator.getIndex()
  }

  /** Asks the docgen service for `componentId` again, which stores the answer where the browser reads it. */
  private async extract(componentId: string): Promise<void> {
    try {
      await (
        await this.extractWith()
      )(componentId)
    } catch (error) {
      logger.debug(`[caliper-docgen] ${componentId} not asked again: ${(error as Error).message}`)
    }
  }

  /**
   * Documents the opening story's component before a browser asks: that starts the docgen worker, the session and the
   * checker, and stores the answer where the browser reads it. Then the components after it, unless a browser already
   * shows a story. Best effort: a failure costs the first story the time it would have saved.
   */
  async start(): Promise<void> {
    if (this.count <= 0) return
    try {
      const extract = await this.extractWith()
      const opening = await prefetch({ index: () => this.index(), extract })
      if (opening !== undefined && this.open.size === 0) await this.prefetchAfter(opening)
    } catch (error) {
      logger.debug(`[caliper-docgen] prefetch stopped: ${(error as Error).message}`)
    }
  }

  /** Whether any browser shows a story of `componentId`. */
  private onScreen(componentId: string): boolean {
    return [...this.open.values()].includes(componentId)
  }

  /**
   * The browser `client` (a tab's manager or preview, by its channel's sender id) shows a story of `storyId`'s
   * component, in place of what it showed before.
   */
  shown(storyId: string, client = ''): void {
    const componentId = componentOf(storyId)
    if (this.open.get(client) === componentId) return
    this.open.set(client, componentId)
    if (this.answeredAway.delete(componentId)) void this.extract(componentId)
    void this.prefetchAfter(componentId)
  }

  /** Has the provider resolve the `prefetchStories - 1` components after `componentId` in the background. Best effort. */
  private async prefetchAfter(componentId: string): Promise<void> {
    if (this.count <= 1) return
    try {
      const entries = neighbours(await this.index(), componentId, this.count - 1)
      // Even an empty list: it replaces what the provider still has queued for the story the browser left.
      this.link.post({ type: 'prefetch', entries })
    } catch (error) {
      logger.debug(`[caliper-docgen] prefetch after ${componentId} stopped: ${(error as Error).message}`)
    }
  }
}

const devServers = new WeakMap<object, DevServer>()

/**
 * One per dev server, which Storybook's channel stands for: it calls `experimental_serverChannel` and
 * `experimental_devServer` once each, with a fresh presets wrapper each time but the same channel.
 */
function devServerFor(options: DevServerOptions): DevServer {
  const key = options.channel ?? options.presets
  let server = devServers.get(key)
  if (!server) {
    server = new DevServer(options, channel())
    devServers.set(key, server)
  }
  return server
}

/** Follows the story the browser shows, which reaches the dev server over Storybook's channel. */
export const experimental_serverChannel = <Channel extends ServerChannel>(
  serverChannel: Channel,
  options: DevServerOptions,
): Channel => {
  const server = devServerFor(options)
  for (const event of STORY_SHOWN) {
    // Storybook's channel calls a listener with the event as `this`, whose `from` names the sending browser channel.
    serverChannel.on(event, function storyShown(this: { from?: unknown } | undefined, shown?: { storyId?: unknown }) {
      if (typeof shown?.storyId !== 'string') return
      server.shown(shown.storyId, typeof this?.from === 'string' ? this.from : '')
    })
  }
  return serverChannel
}

/**
 * When the dev server starts, before a browser has asked, documents the opening story's component through Storybook's
 * docgen service (`prefetchStories`). Not awaited.
 */
export const experimental_devServer = <App>(app: App, options: DevServerOptions): App => {
  void devServerFor(options).start()
  return app
}
