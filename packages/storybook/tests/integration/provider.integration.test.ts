// The provider against the real native binding, in a worker thread like Storybook's docgen worker. Run via
// `pnpm test:integration`, which builds `dist` first.
import { cpSync, mkdtempSync, readFileSync, rmSync, utimesSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { Worker } from 'node:worker_threads'
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

function request(title: string): Promise<Record<string, any>> {
  return new Promise((resolve, reject) => {
    worker.once('message', resolve)
    worker.once('error', reject)
    const entry = {
      type: 'story',
      subtype: 'story',
      id: `${title.toLowerCase()}--basic`,
      name: 'Basic',
      title,
      importPath: `./src/stories/${title}.stories.tsx`,
      tags: [],
    }
    // oxlint-disable-next-line unicorn/require-post-message-target-origin -- a worker has no origin
    worker.postMessage(entry)
  })
}

describe.skipIf(!bindingAvailable)('provider in a worker thread', () => {
  beforeAll(() => {
    project = mkdtempSync(join(tmpdir(), 'caliper-sb-'))
    cpSync(fixture, project, { recursive: true })
    // Workers share the process's working directory, which Storybook sets to the project root.
    process.chdir(project)
    worker = new Worker(new URL('fixtures/provider-worker.mjs', import.meta.url))
  })

  afterAll(async () => {
    await worker?.terminate()
    process.chdir(cwd)
    rmSync(project, { recursive: true, force: true })
  })

  it('answers with argTypes from the session', async () => {
    const payload = await request('Button')
    expect(payload.id).toBe('button')
    expect(payload.description).toBe('A button for actions.')
    expect(Object.keys(payload.argTypes).toSorted()).toEqual(
      ['count', 'disabled', 'icon', 'label', 'onClick', 'primary', 'size', 'variant'].toSorted(),
    )
    expect(payload.argTypes.variant).toEqual({
      name: 'variant',
      description: 'Visual weight.',
      type: {
        required: false,
        name: 'enum',
        raw: '"primary" | "secondary" | "ghost"',
        value: ['primary', 'secondary', 'ghost'],
      },
      table: { type: { summary: '"primary" | "secondary" | "ghost"' }, defaultValue: { summary: "'primary'" } },
    })
    expect(payload.argTypes.label.type).toEqual({ required: true, name: 'string' })
  })

  it('answers with an edit saved since the last request', async () => {
    const file = join(project, 'src', 'components', 'Button.tsx')
    writeFileSync(
      file,
      readFileSync(file, 'utf8').replace(
        '  primary?: boolean\n',
        '  primary?: boolean\n  /** On hover. */\n  tooltip?: string\n',
      ),
    )
    const later = new Date(Date.now() + 10_000)
    utimesSync(file, later, later)
    const payload = await request('Button')
    expect(payload.argTypes.tooltip).toEqual({
      name: 'tooltip',
      description: 'On hover.',
      type: { required: false, name: 'string' },
      table: { type: { summary: 'string' } },
    })
  })

  it('documents subcomponents', async () => {
    const payload = await request('List')
    expect(Object.keys(payload.argTypes).toSorted()).toEqual(['children', 'ordered'])
    expect(Object.keys(payload.subcomponents.ListItem.argTypes).toSorted()).toEqual(['bullet', 'children', 'selected'])
  })
})
