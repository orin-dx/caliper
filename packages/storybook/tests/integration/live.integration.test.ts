// The checker's answer replacing a structural one in the story on screen, against the real native binding and
// Storybook's own service registry: the preset's dev server and the built provider in a worker thread, as Storybook's
// docgen worker runs it, talking over their channel. Run via `pnpm test:integration`, which builds `dist` first.
import { createRequire } from 'node:module'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { Worker } from 'node:worker_threads'
import { SET_CURRENT_STORY } from 'storybook/internal/core-events'
import { experimental_defineService, experimental_registerService, getService } from 'storybook/internal/core-server'
import type { DocgenPayload, IndexEntry, StoryIndex } from 'storybook/internal/types'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'

let bindingAvailable = true
try {
  createRequire(import.meta.url)('caliper-docgen-napi')
} catch (error) {
  if (process.env.NAPI_BINARY_REQUIRED === 'true') throw error
  bindingAvailable = false
}

const checkerProject = fileURLToPath(new URL('../fixtures/checker', import.meta.url))
const badge: IndexEntry = {
  type: 'story',
  subtype: 'story',
  id: 'badge--basic',
  name: 'Basic',
  title: 'Badge',
  importPath: join(checkerProject, 'src', 'stories', 'Badge.stories.tsx'),
}
const index: StoryIndex = { v: 5, entries: { [badge.id]: badge } }

/** Each request the provider in `worker` was asked, answered in order: the worker answers one message at a time. */
function asker(worker: Worker) {
  const asked: string[] = []
  const extract = (entry: IndexEntry) =>
    new Promise<DocgenPayload>((resolve, reject) => {
      asked.push(entry.id)
      worker.once('message', resolve)
      worker.once('error', reject)
      // oxlint-disable-next-line unicorn/require-post-message-target-origin -- a worker has no origin
      worker.postMessage(entry)
    })
  return { asked, extract }
}

const anything = { '~standard': { version: 1, vendor: 'test', validate: (value: unknown) => ({ value }) } } as const

const workerUrl = new URL('fixtures/provider-worker.mjs', import.meta.url)
const srcDirs = [join(checkerProject, 'src')]

describe.skipIf(!bindingAvailable)('the checker replacing a structural answer on screen', () => {
  // Never waits for the checker: every first answer is structural, and the checker's must arrive afterwards.
  const live = new Worker(workerUrl, { workerData: { srcDirs, semanticWaitMs: 0, semanticWaitEmptyMs: 0 } })
  // A fresh session that waits for the checker: what the replaced answer must equal.
  const fresh = new Worker(workerUrl, { workerData: { srcDirs, semanticWaitEmptyMs: 30_000 } })
  const { asked, extract } = asker(live)

  beforeAll(() => {
    const definition = experimental_defineService({
      id: 'core/docgen',
      internal: true,
      initialState: { components: {} as Record<string, DocgenPayload> },
      queries: {
        docgen: {
          input: anything,
          output: anything,
          handler: (input: { id: string }, ctx) => ctx.self.state.components[input.id],
        },
      },
      commands: { extractDocgen: { input: anything, output: anything } },
    })
    experimental_registerService(definition, {
      commands: {
        extractDocgen: {
          handler: async (input: { id: string }, ctx) => {
            const payload = await extract(badge)
            ctx.self.setState((state) => {
              state.components[input.id] = payload
            })
            return payload
          },
        },
      },
    })
  })

  afterAll(async () => {
    await Promise.all([live.terminate(), fresh.terminate()])
  })

  it("stores the structural answer first, then the checker's once, equal to a fresh session's", async () => {
    const { experimental_devServer, experimental_serverChannel } = await import('../../dist/preset.js')
    const listeners = new Map<string, (payload?: { storyId?: unknown }) => void>()
    const serverChannel = {
      on: (event: string, listener: (payload?: { storyId?: unknown }) => void) => listeners.set(event, listener),
    }
    const options = { srcDirs, presets: { apply: async () => ({ getIndex: async () => index }) } }
    experimental_serverChannel(serverChannel, options)
    experimental_devServer({}, options)

    const docgen = getService('core/docgen', { internal: true })
    await expect.poll(() => docgen.queries.docgen.get({ id: 'badge' }), { timeout: 20_000 }).toBeDefined()
    const structural = docgen.queries.docgen.get({ id: 'badge' })!
    expect(asked).toEqual(['badge--basic'])
    // The browser opens the story; the checker's answer is then asked for and stored, without the browser asking.
    listeners.get(SET_CURRENT_STORY)!({ storyId: 'badge--basic' })

    const checked = await asker(fresh).extract(badge)
    expect(Object.keys(checked.argTypes ?? {}).toSorted()).toEqual(['label', 'tone'])
    await expect
      .poll(() => docgen.queries.docgen.get({ id: 'badge' })?.argTypes, { timeout: 30_000 })
      .toEqual(checked.argTypes)
    expect(structural.argTypes).not.toEqual(checked.argTypes)
    expect(docgen.queries.docgen.get({ id: 'badge' })).toEqual(checked)

    // Once: no further request, however long the story stays open or the browser says it shows it.
    listeners.get(SET_CURRENT_STORY)!({ storyId: 'badge--basic' })
    await new Promise((resolve) => setTimeout(resolve, 500))
    expect(asked).toEqual(['badge--basic', 'badge--basic'])
  }, 60_000)
})
