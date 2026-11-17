// The startup prefetch against the real native binding and Storybook's own service registry: the preset asks the
// `core/docgen` service, whose command runs the built provider in a worker thread as Storybook's docgen worker does.
// Run via `pnpm test:integration`, which builds `dist` first.
import { cpSync, mkdtempSync, rmSync } from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { Worker } from 'node:worker_threads'
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

const fixture = fileURLToPath(new URL('../fixtures/project', import.meta.url))
const cwd = process.cwd()
let project: string
let worker: Worker
/** The importPath of every request the provider in the worker was asked, in order. */
const asked: string[] = []

function story(title: string): IndexEntry {
  const id = `${title.toLowerCase()}--basic`
  return { type: 'story', subtype: 'story', id, name: 'Basic', title, importPath: `./src/stories/${title}.stories.tsx` }
}

const index: StoryIndex = {
  v: 5,
  entries: Object.fromEntries(
    ['Button', 'Card', 'List'].map((title) => [`${title.toLowerCase()}--basic`, story(title)]),
  ),
}

/** One request to the provider in the worker, answered in order: the worker answers one message at a time. */
function extract(entry: IndexEntry): Promise<DocgenPayload> {
  return new Promise((resolve, reject) => {
    asked.push(entry.importPath)
    worker.once('message', resolve)
    worker.once('error', reject)
    // oxlint-disable-next-line unicorn/require-post-message-target-origin -- a worker has no origin
    worker.postMessage(entry)
  })
}

/** A standard schema that accepts any value, for the service's inputs and outputs. */
const anything = { '~standard': { version: 1, vendor: 'test', validate: (value: unknown) => ({ value }) } } as const

describe.skipIf(!bindingAvailable)('startup prefetch through the docgen service', () => {
  beforeAll(() => {
    project = mkdtempSync(join(tmpdir(), 'caliper-sb-prefetch-'))
    cpSync(fixture, project, { recursive: true })
    process.chdir(project)
    worker = new Worker(new URL('fixtures/provider-worker.mjs', import.meta.url))
    const byComponent = new Map(Object.values(index.entries).map((entry) => [entry.id.split('--')[0]!, entry]))
    // `core/docgen` as Storybook registers it: the command runs the provider and stores its answer under the id.
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
            const payload = await extract(byComponent.get(input.id)!)
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
    await worker?.terminate()
    process.chdir(cwd)
    rmSync(project, { recursive: true, force: true })
  })

  it("stores the opening story's answer before a browser asks, and answers the browser the same", async () => {
    const { experimental_devServer } = await import('../../dist/preset.js')
    const app = { name: 'app' }
    const options = { srcDirs: ['src'], presets: { apply: async () => ({ getIndex: async () => index }) } }
    expect(await experimental_devServer(app, options)).toBe(app)

    const docgen = getService('core/docgen', { internal: true })
    await expect.poll(() => docgen.queries.docgen.get({ id: 'button' })?.argTypes, { timeout: 10_000 }).toBeDefined()
    const stored = docgen.queries.docgen.get({ id: 'button' })!
    expect(Object.keys(stored.argTypes!).toSorted()).toEqual(
      ['count', 'disabled', 'icon', 'label', 'onClick', 'primary', 'size', 'variant'].toSorted(),
    )
    expect(docgen.queries.docgen.get({ id: 'card' })).toBeUndefined()
    expect(asked).toEqual(['./src/stories/Button.stories.tsx'])

    const browser = await docgen.commands.extractDocgen({ id: 'button' })
    expect(browser?.argTypes).toEqual(stored.argTypes)
    expect(asked).toEqual(['./src/stories/Button.stories.tsx', './src/stories/Button.stories.tsx'])
  })
})
