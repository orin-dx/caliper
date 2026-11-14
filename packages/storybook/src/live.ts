// What the dev server (main thread) and the docgen provider (Storybook's docgen worker thread) tell each other. Storybook
// gives a docgen provider no way to reach the browser, and the dev server no way to reach the provider, so the two talk
// over a `BroadcastChannel` of this name: one process, two threads. The lifecycle of an answer is decided in the dev
// server (`lifecycle.ts`); the provider reports what it answered and does what the dev server asks.

import { BroadcastChannel } from 'node:worker_threads'
import type { IndexEntry } from 'storybook/internal/types'

const CHANNEL = 'caliper-docgen-storybook'

/** What the dev server asks of a provider: every end hears every message, so each names the provider it is for. */
export type ToProvider =
  /** Wait for the checker on `keys` at `revision`, and say how it went (`settled`). */
  | { type: 'watch'; provider: string; component: string; revision: number; keys: string[] }
  /** Resolve these stories' components in the background, in this order, in place of what is still queued. */
  | { type: 'prefetch'; provider: string | undefined; entries: IndexEntry[] }

/** What a provider tells the dev server. */
export type ToDevServer =
  /** It answered for `component` at `revision`, with these keys still awaiting the checker; the service stores it. */
  | { type: 'answer'; provider: string; component: string; revision: number; pending: string[] }
  /** The checker settled `keys` (`ok`: it answered) watched at `revision`; the files are at `revisionNow`. */
  | { type: 'settled'; component: string; revision: number; keys: string[]; ok: boolean; revisionNow: number }

/** One end of the channel. */
export interface Link<Out, In> {
  post(message: Out): void
  listen(receive: (message: In) => void): void
}

/** One end of the named channel, which never keeps its thread alive. */
export function channel<Out, In>(): Link<Out, In> {
  const port = new BroadcastChannel(CHANNEL)
  port.unref()
  return {
    post: (message) => port.postMessage(message),
    listen: (receive) => {
      port.addEventListener('message', (event) => receive((event as MessageEvent<In>).data))
    },
  }
}

/** The component a story or docs id belongs to: `button--primary` is `button`'s. */
export function componentOf(storyId: string): string {
  return storyId.split('--')[0]!
}
