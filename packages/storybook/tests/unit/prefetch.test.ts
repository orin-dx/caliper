import type { IndexEntry, StoryIndex } from 'storybook/internal/types'
import { describe, expect, it } from 'vitest'
import { openingComponents, prefetch } from '../../src/prefetch.js'

function story(id: string, subtype: 'story' | 'test' = 'story'): IndexEntry {
  const title = id.split('--')[0]!
  return { type: 'story', subtype, id, name: id, title, importPath: `./src/${title}.stories.tsx` }
}

function docs(id: string): IndexEntry {
  const title = id.split('--')[0]!
  return { type: 'docs', id, name: 'Docs', title, importPath: `./src/${title}.mdx`, storiesImports: [] }
}

function index(...entries: IndexEntry[]): StoryIndex {
  return { v: 5, entries: Object.fromEntries(entries.map((entry) => [entry.id, entry])) }
}

const stories = index(
  docs('intro--docs'),
  story('button--primary'),
  story('button--secondary'),
  story('card--play', 'test'),
  story('card--basic'),
  story('menu--basic'),
)

describe('openingComponents', () => {
  it('takes the first components the stories show, once each, in index order', () => {
    expect(openingComponents(stories, 1)).toEqual(['button'])
    expect(openingComponents(stories, 2)).toEqual(['button', 'card'])
    expect(openingComponents(stories, 9)).toEqual(['button', 'card', 'menu'])
  })

  it('takes none for a count of 0 or an index without stories', () => {
    expect(openingComponents(stories, 0)).toEqual([])
    expect(openingComponents(index(docs('intro--docs')), 3)).toEqual([])
  })
})

describe('prefetch', () => {
  it('asks for one component after the other, each once its predecessor answered', async () => {
    const events: string[] = []
    const asked = await prefetch(
      {
        index: async () => stories,
        extract: async (id) => {
          events.push(`start ${id}`)
          await new Promise((resolve) => setTimeout(resolve, 5))
          events.push(`end ${id}`)
        },
      },
      2,
    )
    expect(asked).toEqual(['button', 'card'])
    expect(events).toEqual(['start button', 'end button', 'start card', 'end card'])
  })

  it('stops at the first failure', async () => {
    const asked: string[] = []
    const failing = prefetch(
      {
        index: async () => stories,
        extract: async (id) => {
          asked.push(id)
          throw new Error(`no ${id}`)
        },
      },
      3,
    )
    await expect(failing).rejects.toThrow('no button')
    expect(asked).toEqual(['button'])
  })
})
