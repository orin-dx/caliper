import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { DocgenPayload, IndexEntry } from 'storybook/internal/types'
import { logger } from 'storybook/internal/node-logger'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
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

  it('asks the next provider when the session has no such component', async () => {
    const fake = binding()
    delete fake.output.components!.Card
    expect(await request(fake, story('Card'))).toEqual({ payload: NEXT, nextCalls: 1 })
  })

  it('asks the next provider for a degraded component unless fallback is never', async () => {
    const fake = binding()
    fake.output.diagnostics = [warning(['Button'])]
    expect(await request(fake, story('Button'))).toEqual({ payload: NEXT, nextCalls: 1 })
    const { payload, nextCalls } = await request(fake, story('Button'), { fallback: 'never' })
    expect(nextCalls).toBe(0)
    expect(payload?.argTypes).toEqual({ label: labelArgType })
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

  it('passes a docs-only entry straight on without starting a session', async () => {
    const fake = binding()
    const docs = { ...story('Intro', './src/Intro.mdx'), type: 'docs', storiesImports: [] } as unknown as IndexEntry
    expect(await request(fake, docs)).toEqual({ payload: NEXT, nextCalls: 1 })
    expect(fake.initializations).toBe(0)
  })
})
