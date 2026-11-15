// The provider's side of the answer lifecycle (`lifecycle.ts`): it reports each answer to the dev server, which decides
// everything, and carries out what the dev server asks of it: waiting for the checker, and resolving the next stories'
// components in the background. It holds no lifecycle state, only the queue of stories to resolve.

import { randomUUID } from 'node:crypto'
import { logger } from 'storybook/internal/node-logger'
import type { IndexEntry } from 'storybook/internal/types'
import type { Link, ToDevServer, ToProvider } from './live.js'
import type { DocgenSession } from './session.js'

/**
 * How long a watch waits for the checker. The checker answers a story someone opened before any background work, so
 * only a checker still starting (seconds on a large project) comes near this; past it, the watch reports a failure and
 * the next request asks again.
 */
const CHECKER_WAIT_MS = 60_000

/** What the background work needs of the provider. */
export interface BackgroundHost {
  session(): Promise<DocgenSession>
  /** The session keys of the components `entry`'s story file names, the session brought up to date first. */
  keysOf(entry: IndexEntry): Promise<string[]>
}

export class Background {
  /** Names this provider in the messages, since every end of the channel hears every other. */
  readonly id = randomUUID()
  private readonly host: BackgroundHost
  private readonly link: Link<ToDevServer, ToProvider>
  private queue: IndexEntry[] = []
  private prefetching = false

  constructor(host: BackgroundHost, link: Link<ToDevServer, ToProvider>) {
    this.host = host
    this.link = link
    link.listen((message) => {
      if (message.type === 'watch' && message.provider === this.id) void this.watch(message)
      else if (message.type === 'prefetch' && (message.provider ?? this.id) === this.id) {
        this.queue = [...message.entries]
        void this.drain()
      }
    })
  }

  /**
   * Tells the dev server what this provider answered for a component, the service storing it, with the revision the
   * session has seen by now: an edit seen while the extraction ran makes the answer stale.
   */
  async answered(answer: { component: string; revision: number; pending: string[] }): Promise<void> {
    const revisionNow = Math.max(answer.revision, await this.revision())
    this.link.post({ type: 'answer', provider: this.id, ...answer, revisionNow })
  }

  /** The revision of the files the session has last looked at; 0 while there is no session. */
  async revision(): Promise<number> {
    try {
      const { revision } = await this.host.session()
      return revision
    } catch {
      return 0
    }
  }

  private async watch({ component, revision, keys }: { component: string; revision: number; keys: string[] }) {
    let ok = false
    let revisionNow = revision
    try {
      const session = await this.host.session()
      ok = await session.checkerSettled(keys, CHECKER_WAIT_MS)
      ;({ revision: revisionNow } = session)
    } catch (error) {
      logger.debug(`[caliper-docgen] ${component}: ${(error as Error).message}; the next request asks again`)
    }
    this.link.post({ type: 'settled', component, revision, keys, ok, revisionNow })
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
/** `attempt`'s result, else the next provider's (`fallback`), each awaited here, so either has settled on return. */
export async function settle<T>(attempt: () => Promise<T>, fallback: () => Promise<T>, importPath: string): Promise<T> {
  try {
    return await attempt()
  } catch (error) {
    logger.debug(`[caliper-docgen] ${importPath}: ${(error as Error).message}; asking the next docgen provider`)
    return await fallback()
  }
}

/** What one request answered, for the answer lifecycle: the keys still awaiting the checker, and the revision. */
export interface Answered {
  pending: string[]
  revision: number
}
