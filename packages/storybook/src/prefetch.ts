import { getComponentIdFromEntry, isEligibleStoryEntry } from 'storybook/internal/common'
import type { StoryIndex } from 'storybook/internal/types'

/**
 * The component ids of the first `count` components the stories show, in the index's order: the first is the one
 * Storybook opens when the URL names no story.
 */
export function openingComponents(index: StoryIndex, count: number): string[] {
  const ids: string[] = []
  for (const entry of Object.values(index.entries)) {
    if (ids.length >= count) break
    if (!isEligibleStoryEntry(entry)) continue
    const id = getComponentIdFromEntry(entry)
    if (!ids.includes(id)) ids.push(id)
  }
  return ids
}

/** Where the prefetch reads the stories from and how it asks Storybook's docgen service for one component. */
export interface PrefetchTarget {
  index(): Promise<StoryIndex>
  extract(componentId: string): Promise<unknown>
}

/**
 * Asks the docgen service for the opening components one after another, so each answer is stored where the browser
 * reads it and at most one prefetch request is ahead of a story someone opens. Resolves with the ids it asked for.
 */
export async function prefetch(target: PrefetchTarget, count: number): Promise<string[]> {
  const ids = openingComponents(await target.index(), count)
  // oxlint-disable-next-line no-await-in-loop -- one request at a time, so a story someone opens waits behind one at most
  for (const id of ids) await target.extract(id)
  return ids
}
