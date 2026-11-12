// The provider against the real native binding, in a worker thread like Storybook's docgen worker. Run via
// `pnpm test:integration`, which builds `dist` first.
import { cpSync, mkdtempSync, readFileSync, rmSync, utimesSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { Worker } from 'node:worker_threads'
import type { DocgenPayload } from 'storybook/internal/types'
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

function ask(to: Worker, title: string, importPath: string): Promise<DocgenPayload> {
  return new Promise((resolve, reject) => {
    to.once('message', resolve)
    to.once('error', reject)
    const entry = {
      type: 'story',
      subtype: 'story',
      id: `${title.toLowerCase()}--basic`,
      name: 'Basic',
      title,
      importPath,
      tags: [],
    }
    // oxlint-disable-next-line unicorn/require-post-message-target-origin -- a worker has no origin
    to.postMessage(entry)
  })
}

function request(title: string): Promise<Record<string, any>> {
  return ask(worker, title, `./src/stories/${title}.stories.tsx`)
}

/** The pinned tsrs binary for this machine, as the `tsrs` package selects it; `TSRS_BIN` wins when set. */
async function tsrsBinary(): Promise<string> {
  if (process.env.TSRS_BIN) return process.env.TSRS_BIN
  const pkg = createRequire(import.meta.url).resolve('tsrs/package.json')
  const resolver = (await import(pathToFileURL(join(dirname(pkg), 'lib', 'getExePath.js')).href)) as {
    default: () => string
  }
  return resolver.default()
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

  it('documents the component a default export of Object.assign attaches its statics to', async () => {
    const payload = await request('Menu')
    expect(payload.error).toBeUndefined()
    expect(Object.keys(payload.argTypes)).toEqual(['open'])
  })

  it('documents a subcomponent that is a member of a named import', async () => {
    const payload = await request('Picker')
    expect(Object.keys(payload.argTypes)).toEqual(['children'])
    expect(payload.subcomponents.Option.error).toBeUndefined()
    expect(Object.keys(payload.subcomponents.Option.argTypes)).toEqual(['value'])
  })

  it('documents a subcomponent read through a local binding of the story file', async () => {
    const payload = await request('Parts')
    expect(payload.subcomponents.ListItem.error).toBeUndefined()
    expect(Object.keys(payload.subcomponents.ListItem.argTypes).toSorted()).toEqual(['bullet', 'children', 'selected'])
  })

  it('documents the subcomponents of a meta exported by name as the default', async () => {
    const payload = await request('Exported')
    expect(payload.subcomponents.ListItem.error).toBeUndefined()
    expect(Object.keys(payload.subcomponents.ListItem.argTypes).toSorted()).toEqual(['bullet', 'children', 'selected'])
  })
})

// `Badge`'s props are `ComponentProps<typeof StyledBadge>` of a factory call, as Chakra's `Button` is: the structural
// pass cannot find that component and warns, and the checker reads the props. It stays in the repository, where
// `@types/react` resolves for both passes.
const checkerProject = fileURLToPath(new URL('../fixtures/checker', import.meta.url))

describe.skipIf(!bindingAvailable)('provider with the checker and the default fallback', () => {
  let checked: Worker

  beforeAll(async () => {
    checked = new Worker(new URL('fixtures/provider-worker.mjs', import.meta.url), {
      workerData: {
        srcDirs: [join(checkerProject, 'src')],
        semanticTypes: 'on-demand',
        tscPath: await tsrsBinary(),
        semanticWaitEmptyMs: 10_000,
      },
    })
  })

  afterAll(async () => {
    await checked?.terminate()
  })

  it("answers with the checker's props instead of handing the component to the next provider", async () => {
    const payload = await ask(checked, 'Badge', join(checkerProject, 'src', 'stories', 'Badge.stories.tsx'))
    expect(payload.id).toBe('badge')
    expect(Object.keys(payload.argTypes ?? {}).toSorted()).toEqual(['label', 'tone'])
    expect(payload.argTypes?.tone?.type).toEqual({
      required: false,
      name: 'enum',
      raw: '"info" | "warning"',
      value: ['info', 'warning'],
    })
  })
})
