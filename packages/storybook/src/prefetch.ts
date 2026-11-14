import { getComponentIdFromEntry, isEligibleStoryEntry } from 'storybook/internal/common'
import type { IndexEntry, StoryIndex } from 'storybook/internal/types'

/** One story entry per component the stories show, in the index's order: the sidebar's. */
function componentEntries(index: StoryIndex): IndexEntry[] {
  const seen = new Set<string>()
  const entries: IndexEntry[] = []
  for (const entry of Object.values(index.entries)) {
    if (!isEligibleStoryEntry(entry)) continue
    const id = getComponentIdFromEntry(entry)
    if (seen.has(id)) continue
    seen.add(id)
    entries.push(entry)
  }
  return entries
}

/**
 * The component ids of the first `count` components the stories show, in the index's order: the first is the one
 * Storybook opens when the URL names no story.
 */
export function openingComponents(index: StoryIndex, count: number): string[] {
  return componentEntries(index).slice(0, count).map(getComponentIdFromEntry)
}

/**
 * One story entry each of the `count` components after `componentId` in the sidebar, nearest first: the stories
 * someone looking at `componentId` opens next. None when the index does not show `componentId`.
 */
export function neighbours(index: StoryIndex, componentId: string, count: number): IndexEntry[] {
  if (count <= 0) return []
  const entries = componentEntries(index)
  const at = entries.findIndex((entry) => getComponentIdFromEntry(entry) === componentId)
  return at < 0 ? [] : entries.slice(at + 1, at + 1 + count)
}

/** Where the prefetch reads the stories from and how it asks Storybook's docgen service for one component. */
export interface PrefetchTarget {
  index(): Promise<StoryIndex>
  extract(componentId: string): Promise<unknown>
}

/**
 * Asks the docgen service for the opening component, so its answer is stored where the browser reads it, and resolves
 * with its id; undefined when the index shows no story.
 */
export async function prefetch(target: PrefetchTarget): Promise<string | undefined> {
  const [id] = openingComponents(await target.index(), 1)
  if (id !== undefined) await target.extract(id)
  return id
}
