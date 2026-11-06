import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { DocgenPayload } from 'storybook/internal/types'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import { FakeBinding, entry, rdtDoc } from './fake-binding.js'

// Storybook's resolver throws for the subcomponent of `Picker` only.
vi.mock('storybook/internal/common', async (importOriginal) => {
  const original = await importOriginal<typeof import('storybook/internal/common')>()
  return {
    ...original,
    createMetaComponentResolver: () => {
      const real = original.createMetaComponentResolver()
      return (...args: Parameters<typeof real>) => {
        if (args[0].editor.toString().includes('component: Select.Option')) throw new Error('resolver blew up')
        return real(...args)
      }
    },
  }
})

const { createDocgenProvider } = await import('../../src/provider.js')

const project = fileURLToPath(new URL('../fixtures/project', import.meta.url))
const components = join(project, 'src', 'components')
const cwd = process.cwd()

beforeAll(() => process.chdir(project))
afterAll(() => process.chdir(cwd))

describe('a resolver that throws', () => {
  it('hands the story to the next provider instead of reporting a subcomponent error', async () => {
    const fake = new FakeBinding({ components: { Select: entry('Select', join(components, 'Select.tsx')) } })
    fake.docs = { Select: rdtDoc('Select', ['children']) }
    const next: DocgenPayload = { id: 'next', name: 'next', path: 'next', jsDocTags: {} }
    let nextCalls = 0
    const provider = createDocgenProvider(
      {},
      fake,
    )(async () => {
      nextCalls++
      return next
    })
    const payload = await provider({
      entry: {
        type: 'story',
        subtype: 'story',
        id: 'picker--basic',
        name: 'Basic',
        title: 'Picker',
        importPath: './src/stories/Picker.stories.tsx',
        tags: [],
      },
    })
    expect({ payload, nextCalls }).toEqual({ payload: next, nextCalls: 1 })
  })
})
