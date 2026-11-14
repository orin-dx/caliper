// The lifecycle of a story's answer, from the first structural answer to the checker's (ADR 0022). One pure reducer,
// `step(state, event) -> effects`, holds every fact about it; the dev server (`preset.ts`) and the provider
// (`provider.ts`, `background.ts`) only turn Storybook's events into these events and carry out the effects.
//
// Spec.
//
// An answer is what Storybook's docgen service holds for a component (a story file's `meta.component`, by component
// id): the documentation of its keys (the component and its declared subcomponents) at one revision of the files. The
// binding decides, per key, whether the checker's props are in it or are still to come (`awaitsChecker`); nothing
// here derives that, nor any diagnostic, from anything else. A diagnostic has one owner: the binding's answer.
//
// States of an answer, by its keys still awaiting the checker ("pending") and what became of them:
// - structural-final: nothing pending; or every pending key's checker failed, or settled without improving it, at
//   the answer's revision. Nothing more is done for it.
// - structural-pending: some keys pending and not yet settled at its revision: the checker is watched for them.
// - checker-answered: the checker settled every pending key at its revision: the answer is due a refresh.
// - superseded-by-edit: the checker settled after the files changed: the answer is due a refresh, which answers at
//   the new revision (and may be structural-pending again).
// - degraded: the provider handed the component on; the same states apply to its pending keys, since the refresh may
//   answer where the hand-on did not.
//
// Events: `answer` (the provider produced an answer and the service stores it; the last one is what the service
// holds), `settled` (the checker answered, or failed, the watched keys; with the revision the files are at now),
// `selected` (a client shows a story of a component, in place of what it showed), `closed` (a client went away).
// Prefetch ticks need no state here: each `selected` asks for the neighbours of the client's component, and the
// newest list replaces the queued one (the last client to choose sets it).
//
// Effects: `watch` (wait for the checker on keys at a revision), `refresh` (have the service ask again for a
// component, which stores a new answer), `prefetchAfter` (queue the next stories' components in the background, in
// place of what is queued).
//
// Invariants (checked by tests/unit/lifecycle.test.ts over random event sequences):
// 1. After quiescence, every component a client shows holds an answer at the latest revision with no pending key
//    left but those whose checker failed or could not improve it.
// 2. At most one refresh per answer (component and answer sequence number): a refresh answers anew.
// 3. No flapping: at one revision, a key once answered by the checker is never answered structurally again.
// 4. No refresh for a component no client shows; it is refreshed once a client comes back to it.
// 5. Never a watch for a key at a revision it is already watched at, and at most two per key and revision: a key
//    that settled and comes back pending (its checker stopped, or its answer was not kept) is watched once more.

/** One watch's keys at one revision. */
interface Keys {
  revision: number
  keys: Set<string>
}

interface ComponentState {
  /** The answer the service holds: its sequence number, revision, pending keys and whether it was refreshed. */
  shown?: { seq: number; revision: number; pending: string[]; provider: string; refreshed: boolean }
  /** Keys watched and not yet settled. */
  watching: Keys
  /** Keys whose checker settled (answered or failed) at a revision: never watched again there. */
  settled: Keys
  /** Of those, the keys the checker answered. */
  answered: Keys
  /** Keys watched a second time at a revision, after they settled and came back pending. */
  retried: Keys
  /** The checker answered, or the files changed, while no client showed the component: refresh when one does. */
  due: boolean
}

export interface State {
  /** The component each client shows. */
  clients: Map<string, string>
  components: Map<string, ComponentState>
  /** The last answer's sequence number. */
  seq: number
}

export type Event =
  | { type: 'answer'; provider: string; component: string; revision: number; pending: string[] }
  | { type: 'settled'; component: string; revision: number; keys: string[]; ok: boolean; revisionNow: number }
  | { type: 'selected'; client: string; component: string }
  | { type: 'closed'; client: string }

export type Effect =
  | { type: 'watch'; provider: string; component: string; revision: number; keys: string[] }
  | { type: 'refresh'; component: string }
  | { type: 'prefetchAfter'; component: string }

export function initialState(): State {
  return { clients: new Map(), components: new Map(), seq: 0 }
}

const none = (): Keys => ({ revision: -1, keys: new Set() })

function component(state: State, id: string): ComponentState {
  let found = state.components.get(id)
  if (!found) {
    found = { watching: none(), settled: none(), answered: none(), retried: none(), due: false }
    state.components.set(id, found)
  }
  return found
}

/** `keys` at `revision`: the same set when `at` is at that revision, a fresh one otherwise. */
function at(keys: Keys, revision: number): Set<string> {
  return keys.revision === revision ? keys.keys : new Set()
}

function shownBySomeone(state: State, id: string): boolean {
  return [...state.clients.values()].includes(id)
}

/** Refreshes `id` now if a client shows it, else when one does; once per answer. */
function refresh(state: State, id: string, effects: Effect[]): void {
  const entry = component(state, id)
  if (!entry.shown || entry.shown.refreshed) return
  if (!shownBySomeone(state, id)) {
    entry.due = true
    return
  }
  entry.shown.refreshed = true
  entry.due = false
  effects.push({ type: 'refresh', component: id })
}

/** Applies `event` to `state` (in place) and returns the effects to carry out, in order. */
export function step(state: State, event: Event): Effect[] {
  const effects: Effect[] = []
  switch (event.type) {
    case 'answer': {
      const entry = component(state, event.component)
      state.seq += 1
      entry.shown = {
        seq: state.seq,
        revision: event.revision,
        pending: [...event.pending],
        provider: event.provider,
        refreshed: false,
      }
      entry.due = false
      const settled = at(entry.settled, event.revision)
      const retried = at(entry.retried, event.revision)
      const watching = at(entry.watching, event.revision)
      // A key settled at this revision and pending again was asked again (its checker stopped, or its answer was not
      // kept): watched once more, then it stays structural, so a checker that cannot improve it never loops.
      const unwatched = event.pending.filter((key) => !watching.has(key) && !(settled.has(key) && retried.has(key)))
      if (entry.retried.revision !== event.revision) entry.retried = { revision: event.revision, keys: new Set() }
      for (const key of unwatched) if (settled.has(key)) entry.retried.keys.add(key)
      if (unwatched.length > 0) {
        entry.watching = { revision: event.revision, keys: new Set([...watching, ...unwatched]) }
        effects.push({
          type: 'watch',
          provider: event.provider,
          component: event.component,
          revision: event.revision,
          keys: unwatched,
        })
      }
      break
    }
    case 'settled': {
      const entry = component(state, event.component)
      if (entry.watching.revision === event.revision) for (const key of event.keys) entry.watching.keys.delete(key)
      if (entry.settled.revision !== event.revision) entry.settled = { revision: event.revision, keys: new Set() }
      if (entry.answered.revision !== event.revision) entry.answered = { revision: event.revision, keys: new Set() }
      for (const key of event.keys) {
        entry.settled.keys.add(key)
        if (event.ok) entry.answered.keys.add(key)
      }
      const { shown } = entry
      if (!shown || shown.pending.length === 0) break
      if (event.revisionNow > shown.revision) {
        // Superseded by an edit: the answer shown is of older files; asking again answers at the new revision.
        refresh(state, event.component, effects)
      } else if (event.revision === shown.revision) {
        // Checker-answered once every pending key settled, if the checker answered any of them.
        const settled = at(entry.settled, shown.revision)
        const answered = at(entry.answered, shown.revision)
        const done = shown.pending.every((key) => settled.has(key))
        if (done && shown.pending.some((key) => answered.has(key))) refresh(state, event.component, effects)
      }
      break
    }
    case 'selected': {
      const before = state.clients.get(event.client)
      if (before === event.component) break
      state.clients.set(event.client, event.component)
      if (state.components.get(event.component)?.due) refresh(state, event.component, effects)
      effects.push({ type: 'prefetchAfter', component: event.component })
      break
    }
    case 'closed': {
      state.clients.delete(event.client)
      break
    }
  }
  return effects
}
