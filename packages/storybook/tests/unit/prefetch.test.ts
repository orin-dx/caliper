import type { IndexEntry, StoryIndex } from 'storybook/internal/types'
import { describe, expect, it } from 'vitest'
import { neighbours, openingComponents, prefetch } from '../../src/prefetch.js'

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

describe('neighbours', () => {
  it("takes one story each of the components after a component, nearest first, in the sidebar's order", () => {
    expect(neighbours(stories, 'button', 1)).toEqual([story('card--basic')])
    expect(neighbours(stories, 'button', 5)).toEqual([story('card--basic'), story('menu--basic')])
    expect(neighbours(stories, 'card', 2)).toEqual([story('menu--basic')])
  })

  it('takes none after the last component, for a count of 0, or for a component the stories do not show', () => {
    expect(neighbours(stories, 'menu', 3)).toEqual([])
    expect(neighbours(stories, 'button', 0)).toEqual([])
    expect(neighbours(stories, 'intro', 3)).toEqual([])
  })
})

describe('prefetch', () => {
  it('asks for the opening component and says which it asked for', async () => {
    const asked: string[] = []
    const opening = await prefetch({
      index: async () => stories,
      extract: async (id) => {
        asked.push(id)
      },
    })
    expect(opening).toBe('button')
    expect(asked).toEqual(['button'])
  })

  it('asks for nothing when no story shows a component', async () => {
    const asked: string[] = []
    const opening = await prefetch({
      index: async () => index(docs('intro--docs')),
      extract: async (id) => asked.push(id),
    })
    expect(opening).toBeUndefined()
    expect(asked).toEqual([])
  })

  it('fails with the extraction', async () => {
    const failing = prefetch({
      index: async () => stories,
      extract: async (id) => {
        throw new Error(`no ${id}`)
      },
    })
    await expect(failing).rejects.toThrow('no button')
  })
})
