import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { DocgenPayload, IndexEntry } from 'storybook/internal/types'
import { logger } from 'storybook/internal/node-logger'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import type { CaliperStorybookOptions } from '../../src/index.js'
import type { Link, ToDevServer, ToProvider } from '../../src/live.js'
import { createDocgenProvider } from '../../src/provider.js'
import { FakeBinding, entry, rdtDoc, warning } from './fake-binding.js'

const project = fileURLToPath(new URL('../fixtures/project', import.meta.url))
const components = join(project, 'src', 'components')
const cwd = process.cwd()

beforeAll(() => process.chdir(project))
afterAll(() => process.chdir(cwd))

function story(title: string, importPath = `./src/stories/${title}.stories.tsx`): IndexEntry {
  return {
    type: 'story',
    subtype: 'story',
    id: `${title.toLowerCase()}--basic`,
    name: 'Basic',
    title,
    importPath,
    tags: [],
  }
}

function binding(): FakeBinding {
  const fake = new FakeBinding({
    components: {
      Button: entry('Button', join(components, 'Button.tsx')),
      Card: entry('Card', join(components, 'Card.tsx')),
      List: entry('List', join(components, 'List.tsx')),
      ListItem: entry('ListItem', join(components, 'List.tsx')),
    },
  })
  fake.docs = {
    Button: rdtDoc('Button', ['label']),
    Card: rdtDoc('Card', ['title']),
    List: rdtDoc('List', ['ordered']),
    ListItem: rdtDoc('ListItem', ['bullet']),
  }
  return fake
}

const NEXT: DocgenPayload = { id: 'next', name: 'next', path: 'next', jsDocTags: {} }

async function request(fake: FakeBinding, indexEntry: IndexEntry, options = {}) {
  let nextCalls = 0
  const provider = createDocgenProvider(
    options,
    fake,
  )(async () => {
    nextCalls++
    return NEXT
  })
  return { payload: await provider({ entry: indexEntry }), nextCalls }
}

const labelArgType = {
  name: 'label',
  description: '',
  type: { required: false, name: 'string' },
  table: { type: { summary: 'string' }, jsDocTags: undefined, defaultValue: undefined },
}

describe('createDocgenProvider', () => {
  it('documents the component a story imports through a barrel, without asking the next provider', async () => {
    const { payload, nextCalls } = await request(binding(), story('Button'))
    expect(nextCalls).toBe(0)
    expect(payload).toEqual({
      id: 'button',
      name: 'Button',
      path: './src/stories/Button.stories.tsx',
      description: 'Button docs.',
      summary: undefined,
      jsDocTags: {},
      argTypes: { label: labelArgType },
    })
  })

  it("documents a default import as its file's component", async () => {
    const { payload } = await request(binding(), story('Card'))
    expect(payload?.name).toBe('Card')
    expect(Object.keys(payload?.argTypes ?? {})).toEqual(['title'])
  })

  it('documents subcomponents and prefers the meta JSDoc for the description', async () => {
    const { payload } = await request(binding(), story('List'))
    expect(payload?.description).toBe('Lists of things.')
    expect(payload?.subcomponents).toEqual({
      ListItem: {
        name: 'ListItem',
        path: resolve(components, 'List.tsx'),
        description: 'ListItem docs.',
        summary: undefined,
        jsDocTags: {},
        argTypes: { bullet: { ...labelArgType, name: 'bullet' } },
      },
    })
  })

  it('looks at the files once per story, however many subcomponents it documents', async () => {
    const fake = binding()
    await request(fake, story('Button'))
    expect(fake.looks).toBe(1)
    const listed = binding()
    await request(listed, story('List'))
    expect(listed.looks).toBe(1)
  })

  it('documents a member of a named import as the sub-component with that display name', async () => {
    const fake = binding()
    fake.output.components!['Select.Option'] = entry('Select.Option', join(components, 'Select.tsx'))
    fake.docs['Select.Option'] = rdtDoc('Select.Option', ['value'])
    const { payload, nextCalls } = await request(fake, story('Option'))
    expect(nextCalls).toBe(0)
    expect(Object.keys(payload?.argTypes ?? {})).toEqual(['value'])
  })

  it('documents a subcomponent that is a member of a named import', async () => {
    const fake = binding()
    fake.output.components!.Select = entry('Select', join(components, 'Select.tsx'))
    fake.docs.Select = rdtDoc('Select', ['children'])
    fake.output.components!['Select.Option'] = entry('Select.Option', join(components, 'Select.tsx'))
    fake.docs['Select.Option'] = rdtDoc('Select.Option', ['value'])
    const { payload, nextCalls } = await request(fake, story('Picker'))
    expect(nextCalls).toBe(0)
    expect(Object.keys(payload?.subcomponents?.Option?.argTypes ?? {})).toEqual(['value'])
    expect(payload?.subcomponents?.Option?.error).toBeUndefined()
  })

  it('documents a subcomponent read through a local binding of the story file', async () => {
    const { payload, nextCalls } = await request(binding(), story('Parts'))
    expect(nextCalls).toBe(0)
    expect(payload?.subcomponents?.ListItem?.error).toBeUndefined()
    expect(Object.keys(payload?.subcomponents?.ListItem?.argTypes ?? {})).toEqual(['bullet'])
  })

  it.each(['Exported', 'Factory'])('documents the subcomponents of a %s story meta', async (title) => {
    const { payload, nextCalls } = await request(binding(), story(title))
    expect(nextCalls).toBe(0)
    expect(payload?.subcomponents?.ListItem?.error).toBeUndefined()
    expect(Object.keys(payload?.subcomponents?.ListItem?.argTypes ?? {})).toEqual(['bullet'])
  })

  it('asks the next provider when the session has no such component', async () => {
    const fake = binding()
    delete fake.output.components!.Card
    expect(await request(fake, story('Card'))).toEqual({ payload: NEXT, nextCalls: 1 })
  })

  it('says which components the file has when none matches the import', async () => {
    const debug = vi.spyOn(logger, 'debug').mockImplementation(() => {})
    const fake = binding()
    fake.output.components = {
      Header: entry('Header', join(components, 'Card.tsx')),
      Footer: entry('Footer', join(components, 'Card.tsx')),
    }
    await request(fake, story('Card'))
    expect(debug.mock.calls.map(([message]) => message)).toEqual([
      `[caliper-docgen] ./src/stories/Card.stories.tsx: caliper-docgen found no component default in ${resolve(components, 'Card.tsx')}; its components are Header, Footer; asking the next docgen provider`,
    ])
    debug.mockRestore()
  })

  it('asks the next provider for a degraded component unless fallback is never', async () => {
    const fake = binding()
    fake.output.diagnostics = [warning(['Button'])]
    expect(await request(fake, story('Button'))).toEqual({ payload: NEXT, nextCalls: 1 })
    const { payload, nextCalls } = await request(fake, story('Button'), { fallback: 'never' })
    expect(nextCalls).toBe(0)
    expect(payload?.argTypes).toEqual({ label: labelArgType })
  })

  it("answers for a component the checker resolved, and hands one on when the checker's own warning stays", async () => {
    const checker = (message: string, severity: 'info' | 'warning') => ({
      ...warning(['Button'], severity),
      code: 'SEMANTIC_BACKEND',
      message,
    })
    const answered = binding()
    answered.output.diagnostics = [
      checker('1 component(s) resolved by the TypeScript checker', 'info'),
      checker('1 prop(s) of 1 component(s) keep their structural type', 'info'),
    ]
    const { payload, nextCalls } = await request(answered, story('Button'))
    expect(nextCalls).toBe(0)
    expect(payload?.argTypes).toEqual({ label: labelArgType })

    const stopped = binding()
    stopped.output.diagnostics = [
      checker('the TypeScript checker stopped without answering; the structural props are used', 'warning'),
    ]
    expect(await request(stopped, story('Button'))).toEqual({ payload: NEXT, nextCalls: 1 })
  })

  it('warns once per component handed on, with the reason', async () => {
    const warn = vi.spyOn(logger, 'warn').mockImplementation(() => {})
    const fake = binding()
    fake.output.diagnostics = [warning(['Button'])]
    const provider = createDocgenProvider({}, fake)(async () => NEXT)
    await provider({ entry: story('Button') })
    await provider({ entry: story('Button') })
    expect(warn).toHaveBeenCalledTimes(1)
    expect(warn.mock.calls[0]?.[0]).toBe(
      "[caliper-docgen] Button: Cannot resolve type; Storybook's own docgen answers instead",
    )
    warn.mockRestore()
  })

  it('reads dependencies and lists inherited props unless told not to, and passes the HTML attribute choice on', async () => {
    const seen: unknown[] = []
    const spy = (fake: FakeBinding) => {
      const init = fake.initializeSessionIndex
      fake.initializeSessionIndex = async (id: number, options: unknown) => {
        seen.push(options)
        return init(id, options as never)
      }
      return fake
    }
    await request(spy(binding()), story('Button'))
    expect(seen[0]).toMatchObject({ loadDependencies: true, collapseInheritedProps: 'none', htmlAttributes: undefined })
    await request(spy(binding()), story('Button'), {
      loadDependencies: false,
      htmlAttributes: 'full',
      collapseInheritedProps: 'all',
    })
    expect(seen[1]).toMatchObject({ loadDependencies: false, collapseInheritedProps: 'all', htmlAttributes: 'full' })
  })

  it('warns once, at a level Storybook shows, when the session cannot start, and starts again on the next request', async () => {
    const warn = vi.spyOn(logger, 'warn').mockImplementation(() => {})
    const fake = binding()
    const create = fake.createSession
    fake.createSession = () => {
      throw new Error('no binding for this platform')
    }
    const provider = createDocgenProvider({}, fake)(async () => NEXT)

    expect(await provider({ entry: story('Button') })).toBe(NEXT)
    expect(await provider({ entry: story('Button') })).toBe(NEXT)
    expect(warn.mock.calls).toEqual([
      ["[caliper-docgen] could not start: no binding for this platform; Storybook's own docgen answers instead"],
    ])

    fake.createSession = create
    expect((await provider({ entry: story('Button') }))?.argTypes).toEqual({ label: labelArgType })
    warn.mockRestore()
  })

  it('words a refresh failure of a running session as one, and reports it again after a refresh succeeds', async () => {
    const warn = vi.spyOn(logger, 'warn').mockImplementation(() => {})
    const fake = binding()
    const ensureFresh = fake.ensureFresh
    const provider = createDocgenProvider({}, fake)(async () => NEXT)
    expect((await provider({ entry: story('Button') }))?.argTypes).toEqual({ label: labelArgType })

    fake.ensureFresh = async () => {
      throw new Error('disk gone')
    }
    expect(await provider({ entry: story('Button') })).toBe(NEXT)
    expect(await provider({ entry: story('Button') })).toBe(NEXT)
    fake.ensureFresh = ensureFresh
    expect((await provider({ entry: story('Button') }))?.argTypes).toEqual({ label: labelArgType })
    fake.ensureFresh = async () => {
      throw new Error('disk gone again')
    }
    expect(await provider({ entry: story('Button') })).toBe(NEXT)
    expect(warn.mock.calls).toEqual([
      ["[caliper-docgen] could not refresh: disk gone; Storybook's own docgen answers instead"],
      ["[caliper-docgen] could not refresh: disk gone again; Storybook's own docgen answers instead"],
    ])
    warn.mockRestore()
  })

  it('fails at startup for a fallback or checker that is not a choice, and for the old semanticTypes', () => {
    expect(() => createDocgenProvider({ fallback: 'Degraded' as never }, binding())).toThrow(
      "[caliper-docgen] fallback is 'Degraded', expected 'degraded' or 'never'",
    )
    expect(() => createDocgenProvider({ checker: 'on-demand' as never }, binding())).toThrow(
      "[caliper-docgen] checker is 'on-demand', expected true or false",
    )
    expect(() => createDocgenProvider({ semanticTypes: 'off' } as never, binding())).toThrow(
      "[caliper-docgen] semanticTypes was replaced by checker: the checker is on by default, and 'checker: false' turns it off",
    )
  })

  it('passes a docs-only entry straight on without starting a session', async () => {
    const fake = binding()
    const docs = { ...story('Intro', './src/Intro.mdx'), type: 'docs', storiesImports: [] } as unknown as IndexEntry
    expect(await request(fake, docs)).toEqual({ payload: NEXT, nextCalls: 1 })
    expect(fake.initializations).toBe(0)
  })
})

// The lifecycle itself is tested in lifecycle.test.ts; these test the provider's reports and how it carries out what
// the dev server asks of it.
describe('the provider as an adapter of the answer lifecycle', () => {
  function linked(fake: FakeBinding, options: CaliperStorybookOptions = {}) {
    const posted: ToDevServer[] = []
    let receive: (message: ToProvider) => void = () => {}
    const link: Link<ToDevServer, ToProvider> = {
      post: (message) => posted.push(message),
      listen: (listener) => {
        receive = listener
      },
    }
    const provider = createDocgenProvider(options, fake, link)(async () => NEXT)
    return { provider, posted, say: (message: ToProvider) => receive(message) }
  }

  it("reports every answer with the binding's pending keys, the subcomponents asked before a parent handed on", async () => {
    for (const lazy of [true, false]) {
      const fake = binding()
      // The parent is degraded (handed on) and pending; its subcomponent is pending too.
      fake.output.diagnostics = [warning(['List'])]
      fake.pending.add('List').add('ListItem')
      const { provider, posted } = linked(fake, { lazy })
      expect(await provider({ entry: story('List') })).toEqual(NEXT)
      expect(posted).toEqual([
        {
          type: 'answer',
          provider: expect.any(String),
          component: 'list',
          revision: 1,
          pending: ['ListItem', 'List'],
          revisionNow: 1,
        },
      ])
    }
  })

  it('keeps the newest completion when an older async fallback finishes after a newer request', async () => {
    const fake = binding()
    fake.output.diagnostics = [warning(['Card'])]
    fake.pending.add('Card')
    const posted: ToDevServer[] = []
    const link: Link<ToDevServer, ToProvider> = { post: (message) => posted.push(message), listen: () => {} }
    let finishFallback: (payload: DocgenPayload) => void = () => {}
    const fallback = new Promise<DocgenPayload>((done) => {
      finishFallback = done
    })
    const provider = createDocgenProvider({}, fake, link)(() => fallback)
    // The first request is handed on to a slow provider; the checker answers before it returns.
    const older = provider({ entry: story('Card') })
    await vi.waitFor(() => expect(fake.resolved).toContain('Card'))
    fake.output.diagnostics = []
    fake.pending.clear()
    const newer = await provider({ entry: story('Card') })
    expect(Object.keys(newer!.argTypes!)).toEqual(['title'])
    finishFallback(NEXT)
    // What the service stores from the older request is the newer payload; only the newer one is announced.
    expect(await older).toEqual(newer)
    expect(posted).toEqual([
      { type: 'answer', provider: expect.any(String), component: 'card', revision: 1, pending: [], revisionNow: 1 },
    ])
  })

  it('reports an answer with nothing pending, so the lifecycle knows the structural answer was replaced', async () => {
    const { provider, posted } = linked(binding())
    await provider({ entry: story('Card') })
    expect(posted).toEqual([
      { type: 'answer', provider: expect.any(String), component: 'card', revision: 1, pending: [], revisionNow: 1 },
    ])
  })

  it('carries out a watch addressed to it, and reports the revision the files are at; not one for another', async () => {
    const fake = binding()
    fake.pending.add('Card')
    const { provider, posted, say } = linked(fake)
    await provider({ entry: story('Card') })
    const id = (posted[0] as { provider: string }).provider
    say({ type: 'watch', provider: 'someone-else', component: 'card', revision: 1, keys: ['Card'] })
    fake.settled = Promise.resolve(false)
    say({ type: 'watch', provider: id, component: 'card', revision: 1, keys: ['Card'] })
    await vi.waitFor(() =>
      expect(posted.slice(1)).toEqual([
        { type: 'settled', component: 'card', revision: 1, keys: ['Card'], ok: false, revisionNow: 1 },
      ]),
    )
    expect(fake.settles).toEqual([['Card']])
  })

  it("resolves the named stories' components in the background, in order, the newest list replacing the queue", async () => {
    const fake = binding()
    const { provider, posted, say } = linked(fake)
    await provider({ entry: story('Card') })
    const id = (posted[0] as { provider: string }).provider
    say({ type: 'prefetch', provider: 'someone-else', entries: [story('Card')] })
    say({ type: 'prefetch', provider: id, entries: [story('List'), story('Card')] })
    await vi.waitFor(() => expect(fake.prefetched).toEqual([['List', 'ListItem'], ['Card']]))
    say({ type: 'prefetch', provider: undefined, entries: [story('Button')] })
    await vi.waitFor(() => expect(fake.prefetched).toEqual([['List', 'ListItem'], ['Card'], ['Button']]))
  })
})
