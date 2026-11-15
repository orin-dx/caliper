// The provider's side of the answer lifecycle (`lifecycle.ts`): it reports each answer to the dev server, which decides
// everything, and carries out what the dev server asks of it: waiting for the checker, and resolving the next stories'
// components in the background. It holds no lifecycle state, only the queue of stories to resolve and what it asked
// the checker for ahead of the client (`Prefetched`, decided by `prefetchStep`).
//
// The binding decides each request's place in the checker's queue (ADR 0028): the component a story shows is asked at
// the on-screen class (`sessionComponentAnswers`), the next stories' components at the neighbour class
// (`sessionPrefetch`). This side only says when a prefetch is no longer wanted, since only it sees the queue replaced:
// - A new list replaces the queue (the client navigated away): every key asked for the old list is cancelled
//   (`sessionCancel`). The binding never cancels a request someone waits for, so a key the client now shows stays.
// - A key of an old list asked after the replacement (its story was being resolved) is cancelled once asked.
// - An edit drops every queued request nobody waits for (the binding does this itself): once the provider sees the
//   newer revision, the keys asked for the current list are asked again.

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
  private readonly prefetched = initialPrefetched()

  constructor(host: BackgroundHost, link: Link<ToDevServer, ToProvider>) {
    this.host = host
    this.link = link
    link.listen((message) => {
      if (message.type === 'watch' && message.provider === this.id) void this.watch(message)
      else if (message.type === 'prefetch' && (message.provider ?? this.id) === this.id) {
        this.queue = [...message.entries]
        this.dispatch({ type: 'replaced' })
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
    this.dispatch({ type: 'seen', revision: revisionNow })
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
      this.dispatch({ type: 'seen', revision: revisionNow })
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
    const { generation } = this.prefetched
    await this.ask(await this.host.keysOf(entry), generation)
  }

  /** Asks the checker for `keys` at the neighbour class, for the list of `generation`, and reports it. */
  private async ask(keys: string[], generation: number): Promise<void> {
    if (keys.length === 0) return
    const session = await this.host.session()
    // The revision before the ask: an edit applied while it runs makes the ask look older, so it is asked again.
    const { revision } = session
    this.dispatch({ type: 'asking', generation, keys })
    try {
      await session.prefetch(keys)
    } finally {
      this.dispatch({ type: 'asked', generation, keys, revision })
    }
  }

  /** Applies `event` to what was prefetched and carries out its effects. Best effort, like the prefetch. */
  private dispatch(event: PrefetchEvent): void {
    for (const effect of prefetchStep(this.prefetched, event)) void this.carryOut(effect)
  }

  private async carryOut(effect: PrefetchEffect): Promise<void> {
    try {
      if (effect.type === 'ask') await this.ask(effect.keys, this.prefetched.generation)
      else {
        const session = await this.host.session()
        session.cancel(effect.keys)
      }
    } catch (error) {
      logger.debug(
        `[caliper-docgen] prefetch ${effect.type} of ${effect.keys.join(', ')} stopped: ${(error as Error).message}`,
      )
    }
  }
}

/** What the provider asked the checker for ahead of the client: the one state `prefetchStep` decides from. */
export interface Prefetched {
  /** Which prefetch list is current: each new list replaces the one before. */
  generation: number
  /** The keys asked for the current list, in order, each once. */
  asked: string[]
  /** The keys being asked for the current list, not reported yet, each as many times as it is being asked. */
  asking: string[]
  /** The newest revision of the files the provider has seen. */
  revision: number
}

export type PrefetchEvent =
  /** A new prefetch list replaced the queue: the client shows another story. */
  | { type: 'replaced' }
  /** `keys` are being asked for the list of `generation`. */
  | { type: 'asking'; generation: number; keys: string[] }
  /** `keys` were asked for the list of `generation`, with the session at `revision` when the ask started. */
  | { type: 'asked'; generation: number; keys: string[]; revision: number }
  /** The provider saw the files at `revision`. */
  | { type: 'seen'; revision: number }

export type PrefetchEffect =
  /** Drop the queued requests for `keys`: nobody will read them. */
  | { type: 'cancel'; keys: string[] }
  /** Ask for `keys` again at the neighbour class, for the current list: an edit dropped them. */
  | { type: 'ask'; keys: string[] }

export function initialPrefetched(): Prefetched {
  return { generation: 0, asked: [], asking: [], revision: 0 }
}

/** Applies `event` to `state` (in place) and returns the effects to carry out, in order. */
export function prefetchStep(state: Prefetched, event: PrefetchEvent): PrefetchEffect[] {
  switch (event.type) {
    case 'replaced': {
      state.generation += 1
      const keys = state.asked
      state.asked = []
      // Those still being asked are cancelled once reported, as of a replaced list.
      state.asking = []
      return keys.length > 0 ? [{ type: 'cancel', keys }] : []
    }
    case 'asking': {
      if (event.generation === state.generation) state.asking.push(...event.keys)
      return []
    }
    case 'asked': {
      if (event.generation !== state.generation) {
        // Asked for a list already replaced; a key the current list asks too stays.
        const keys = event.keys.filter((key) => !state.asked.includes(key) && !state.asking.includes(key))
        return keys.length > 0 ? [{ type: 'cancel', keys }] : []
      }
      for (const key of event.keys) state.asking.splice(state.asking.indexOf(key), 1)
      // The keys asked before an edit this ask saw are asked again; these were asked at its revision.
      const effects = seen(state, event.revision)
      const fresh = event.keys.filter((key) => !state.asked.includes(key))
      state.asked.push(...fresh)
      // Asked at a revision older than one already seen: an edit since dropped them.
      if (event.revision < state.revision && fresh.length > 0) effects.push({ type: 'ask', keys: fresh })
      return effects
    }
    case 'seen': {
      return seen(state, event.revision)
    }
  }
}

/** Records that the files are at `revision`; on an edit, asks again for what the current list asked. */
function seen(state: Prefetched, revision: number): PrefetchEffect[] {
  if (revision <= state.revision) return []
  state.revision = revision
  return state.asked.length > 0 ? [{ type: 'ask', keys: [...state.asked] }] : []
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
