// What the dev server (main thread) and the docgen provider (Storybook's docgen worker thread) tell each other. Storybook
// gives a docgen provider no way to reach the browser, and the dev server no way to reach the provider, so the two talk
// over a `BroadcastChannel` of this name: one process, two threads.

import { BroadcastChannel } from 'node:worker_threads'
import type { IndexEntry } from 'storybook/internal/types'

const CHANNEL = 'caliper-docgen-storybook'

/** What the dev server tells the provider: resolve these stories' components in the background, in this order, in
 * place of what is still queued. */
export interface ToProvider {
  type: 'prefetch'
  entries: IndexEntry[]
}

/**
 * What the provider tells the dev server: `componentId`'s answer went out with structural props, and the checker has
 * since answered every component in it. Asked again, the docgen service stores the checker's answer where the browser
 * reads it. Said at most once per component and revision of the files.
 */
export interface ToDevServer {
  type: 'answered'
  componentId: string
}

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
