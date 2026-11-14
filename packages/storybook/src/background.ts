// The provider's work that no request waits for (ADR 0022): replacing a structural answer once the checker has
// answered, and resolving the next stories' components in the background.

import { logger } from 'storybook/internal/node-logger'
import type { IndexEntry } from 'storybook/internal/types'
import type { Link, ToDevServer, ToProvider } from './live.js'
import type { DocgenSession } from './session.js'

/**
 * How long a component answered with its structural props waits for the checker's answer to replace them. The checker
 * answers a story someone opened before any background work, so only a checker still starting (seconds on a large
 * project) comes near this; past it, the next request asks again.
 */
const CHECKER_WAIT_MS = 60_000

/** What the background work needs of the provider. */
export interface BackgroundHost {
  session(): Promise<DocgenSession>
  /** The session keys of the components `entry`'s story file names, the session brought up to date first. */
  keysOf(entry: IndexEntry): Promise<string[]>
}

/**
 * Replacing a structural answer: one that went out with structural props because the checker had not answered in time
 * is watched until the checker has answered every component in it; then the dev server, if the browser shows that
 * component, has the docgen service ask again, and the provider answers from the checker's kept answer. Said once per
 * component and revision of the files, so an answer the checker could not improve (its answer refused for an edit made
 * meanwhile) is not asked for again until the files change.
 *
 * The prefetch beyond the opening story: the dev server names the stories after the one on screen, and their components
 * are resolved in the background, one story at a time, the checker's answers included. A newer list replaces what is
 * still queued.
 */
export class Background {
  private readonly host: BackgroundHost
  private readonly link: Link<ToDevServer, ToProvider>
  /** The components being watched. */
  private readonly watching = new Set<string>()
  /** The revision each component's structural answer was last said to be replaced at. */
  private readonly replaced = new Map<string, number>()
  private queue: IndexEntry[] = []
  private prefetching = false

  constructor(host: BackgroundHost, link: Link<ToDevServer, ToProvider>) {
    this.host = host
    this.link = link
    link.listen((message) => {
      // Every end of the channel hears every other, another provider's word included.
      if (message.type !== 'prefetch') return
      this.queue = [...message.entries]
      void this.drain()
    })
  }

  /** Watches `componentId`'s answer, which went out structural for the components `awaiting` names (none: nothing to do). */
  async watch(componentId: string, awaiting: string[]): Promise<void> {
    if (awaiting.length === 0 || this.watching.has(componentId)) return
    this.watching.add(componentId)
    try {
      const session = await this.host.session()
      const { revision } = session
      if (this.replaced.get(componentId) === revision) return
      if (!(await session.checkerSettled(awaiting, CHECKER_WAIT_MS))) return
      // An edit since the answer drops the checker's answers; the request that sees the edit answers it.
      if (session.revision !== revision) return
      this.replaced.set(componentId, revision)
      this.link.post({ type: 'answered', componentId })
    } catch (error) {
      logger.debug(`[caliper-docgen] ${componentId}: ${(error as Error).message}; the next request asks again`)
    } finally {
      this.watching.delete(componentId)
    }
  }

  private async drain(): Promise<void> {
    if (this.prefetching) return
    this.prefetching = true
    try {
      while (this.queue.length > 0) {
        const entry = this.queue.shift()!
        try {
          // oxlint-disable-next-line no-await-in-loop -- one story at a time, so a story someone opens waits behind one at most
          await this.prefetchStory(entry)
        } catch (error) {
          logger.debug(`[caliper-docgen] prefetch of ${entry.importPath} stopped: ${(error as Error).message}`)
        }
      }
    } finally {
      this.prefetching = false
    }
  }

  private async prefetchStory(entry: IndexEntry): Promise<void> {
    const keys = await this.host.keysOf(entry)
    const session = await this.host.session()
    await session.prefetch(keys)
  }
}
