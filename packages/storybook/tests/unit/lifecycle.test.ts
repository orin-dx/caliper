// A model-based test of the answer lifecycle (`src/lifecycle.ts`): a simulated Storybook, provider, session and checker
// drive the reducer through random event sequences, then run to quiescence, and the spec's invariants are checked.
// Named sequences replay the review findings the reducer replaced spot fixes for.
import { describe, expect, it } from 'vitest'
import { type Event, initialState, step } from '../../src/lifecycle.js'

/** Each component's keys: the component and its declared subcomponents. */
const COMPONENTS: Record<string, string[]> = { a: ['A', 'A.Item'], b: ['B'], c: ['C', 'C.Part'] }
const CLIENTS = ['tab-1', 'tab-2', 'tab-3']

/** A small seeded generator (mulberry32), so a failing seed replays. */
function generator(seed: number): (n: number) => number {
  let t = seed >>> 0
  return (n) => {
    t = (t + 0x6d2b79f5) >>> 0
    let r = Math.imul(t ^ (t >>> 15), 1 | t)
    r = (r + Math.imul(r ^ (r >>> 7), 61 | r)) ^ r
    return Math.floor((((r ^ (r >>> 14)) >>> 0) / 4294967296) * n)
  }
}

interface Stored {
  seq: number
  revision: number
  checker: Set<string>
}

/** The world the lifecycle runs in, with the adapters' behaviour: what the reducer cannot see, it is told by events. */
class World {
  readonly state = initialState()
  /** The files' revision on disk, and the one the session last looked at. */
  disk = 0
  revision = 0
  /** Keys whose checker answer the session keeps, at `revision`. */
  readonly cache = new Set<string>()
  readonly inflight: { key: string; generation: number }[] = []
  readonly watches: { component: string; revision: number; keys: string[] }[] = []
  readonly tasks: (() => void)[] = []
  readonly clients = new Map<string, string>()
  /** What the docgen service holds per component, and every answer it held, in order. */
  readonly stored = new Map<string, Stored>()
  readonly history = new Map<string, Stored[]>()
  /** Keys whose checker failed or whose watch timed out, at a revision (`key@revision`). */
  readonly failed = new Set<string>()
  readonly refreshes: { component: string; seq: number; visible: boolean }[] = []
  readonly watchCount = new Map<string, number>()

  private visible(component: string): boolean {
    return [...this.clients.values()].includes(component)
  }

  dispatch(event: Event): void {
    for (const effect of step(this.state, event)) {
      if (effect.type === 'watch') {
        this.watches.push({ component: effect.component, revision: effect.revision, keys: effect.keys })
        for (const key of effect.keys) {
          const at = `${key}@${effect.revision}`
          this.watchCount.set(at, (this.watchCount.get(at) ?? 0) + 1)
        }
      } else if (effect.type === 'refresh') {
        const seq = this.state.components.get(effect.component)!.shown!.seq
        this.refreshes.push({ component: effect.component, seq, visible: this.visible(effect.component) })
        this.tasks.push(() => this.answer(effect.component))
      }
    }
    this.deliverReady()
  }

  /** The provider answers for `component` after a look at the files; the service stores it. */
  answer(component: string): void {
    if (this.disk > this.revision) {
      this.revision = this.disk
      this.cache.clear()
    }
    const checker = new Set<string>()
    const pending: string[] = []
    for (const key of COMPONENTS[component]!) {
      if (this.cache.has(key)) checker.add(key)
      else {
        if (!this.inflight.some((each) => each.key === key && each.generation === this.revision)) {
          this.inflight.push({ key, generation: this.revision })
        }
        pending.push(key)
      }
    }
    this.history.set(component, [...(this.history.get(component) ?? []), { seq: 0, revision: this.revision, checker }])
    this.stored.set(component, { seq: 0, revision: this.revision, checker })
    this.dispatch({ type: 'answer', provider: 'p', component, revision: this.revision, pending })
  }

  /** A checker request ends: kept when it answered and no look dropped it since; a failure is not kept. */
  complete(index: number, ok: boolean): void {
    const [request] = this.inflight.splice(index, 1)
    if (ok && request!.generation === this.revision) this.cache.add(request!.key)
    if (!ok) this.failed.add(`${request!.key}@${request!.generation}`)
    this.deliverReady()
  }

  /** A watch resolves once no request is in flight for its keys, as `sessionCheckerSettled` does. */
  private deliverReady(): void {
    for (let i = this.watches.length - 1; i >= 0; i -= 1) {
      const watch = this.watches[i]!
      if (this.inflight.some((request) => watch.keys.includes(request.key))) continue
      this.watches.splice(i, 1)
      this.tasks.push(() => this.dispatch({ type: 'settled', ...watch, ok: true, revisionNow: this.revision }))
    }
  }

  timeout(index: number): void {
    const [watch] = this.watches.splice(index, 1)
    for (const key of watch!.keys) this.failed.add(`${key}@${watch!.revision}`)
    this.tasks.push(() => this.dispatch({ type: 'settled', ...watch!, ok: false, revisionNow: this.revision }))
  }

  /** A client shows a story of `component`; the browser asks the service when it holds nothing for it. */
  select(client: string, component: string): void {
    this.clients.set(client, component)
    this.dispatch({ type: 'selected', client, component })
    if (!this.stored.has(component)) this.tasks.push(() => this.answer(component))
  }

  close(client: string): void {
    this.clients.delete(client)
    this.dispatch({ type: 'closed', client })
  }

  /** A saved edit; Storybook's module graph has the service ask again for every component it holds. */
  edit(): void {
    this.disk += 1
    for (const component of this.stored.keys()) this.tasks.push(() => this.answer(component))
  }

  run(index: number): void {
    const [task] = this.tasks.splice(index, 1)
    task!()
  }

  /** Every request answers and every task runs, until nothing is left. */
  quiesce(): void {
    for (let i = 0; i < 10_000; i += 1) {
      if (this.tasks.length > 0) this.run(0)
      else if (this.inflight.length > 0) this.complete(0, true)
      else return
    }
    throw new Error('no quiescence')
  }

  /** The spec's invariants after quiescence: each one broken, described; none when all hold. */
  violations(): string[] {
    const broken: string[] = []
    for (const component of new Set(this.clients.values())) {
      const shown = this.stored.get(component)!
      // 1. The latest revision, every key the checker's but those it failed at that revision.
      if (shown.revision !== this.revision) broken.push(`1: ${component} shows revision ${shown.revision}`)
      for (const key of COMPONENTS[component]!) {
        if (!this.failed.has(`${key}@${shown.revision}`) && !shown.checker.has(key)) {
          broken.push(`1: ${component} shows ${key} structural`)
        }
      }
    }
    // 2. At most one refresh per answer.
    const seqs = this.refreshes.map(({ component, seq }) => `${component}#${seq}`)
    if (new Set(seqs).size !== seqs.length) broken.push(`2: refreshes ${seqs.join(' ')}`)
    // 3. No flapping: at one revision, a key the checker answered is never structural again.
    for (const answers of this.history.values()) {
      for (let i = 1; i < answers.length; i += 1) {
        const [before, after] = [answers[i - 1]!, answers[i]!]
        if (before.revision !== after.revision) continue
        for (const key of before.checker) if (!after.checker.has(key)) broken.push(`3: ${key} flapped`)
      }
    }
    // 4. No refresh for a component no client shows.
    for (const each of this.refreshes) if (!each.visible) broken.push(`4: ${each.component} refreshed unseen`)
    // 5. At most two watches per key and revision.
    for (const [at, count] of this.watchCount) if (count > 2) broken.push(`5: ${at} watched ${count} times`)
    return broken
  }
}

describe('the answer lifecycle, over random event sequences', () => {
  it('keeps every invariant after quiescence', () => {
    const names = Object.keys(COMPONENTS)
    for (let seed = 1; seed <= 400; seed += 1) {
      const pick = generator(seed)
      const world = new World()
      const clients = CLIENTS.slice(0, 1 + pick(3))
      for (let n = 0; n < 60; n += 1) {
        const roll = pick(100)
        if (roll < 22) world.select(clients[pick(clients.length)]!, names[pick(names.length)]!)
        else if (roll < 27 && clients.length > 1) world.close(clients[pick(clients.length)]!)
        else if (roll < 35) world.edit()
        else if (roll < 55 && world.inflight.length > 0) world.complete(pick(world.inflight.length), pick(5) > 0)
        else if (roll < 58 && world.watches.length > 0) world.timeout(pick(world.watches.length))
        else if (world.tasks.length > 0) world.run(pick(world.tasks.length))
      }
      world.quiesce()
      expect(world.violations(), `seed ${seed}`).toEqual([])
    }
  })
})

describe('the answer lifecycle, named regressions', () => {
  it('an edit while a watch is pending: the newer revision is refreshed, the older watch says nothing', () => {
    const world = new World()
    world.select('tab-1', 'b')
    world.run(0)
    world.edit()
    world.run(0) // Storybook's re-extract after the edit: structural at the new revision, watched anew
    world.quiesce()
    expect(world.violations()).toEqual([])
    expect(world.stored.get('b')).toMatchObject({ revision: 1, checker: new Set(['B']) })
  })

  it('two clients on different components: an answer refreshes the one any client shows', () => {
    const world = new World()
    world.select('tab-1', 'a')
    world.select('tab-2', 'b')
    world.quiesce()
    expect(world.violations()).toEqual([])
    expect(world.refreshes.map(({ component }) => component).toSorted()).toEqual(['a', 'b'])
  })

  it('a client that left: no refresh while away, one when a client comes back', () => {
    const world = new World()
    world.select('tab-1', 'a')
    world.run(0)
    world.select('tab-1', 'b')
    world.quiesce()
    expect(world.refreshes.map(({ component }) => component)).toEqual(['b'])
    world.select('tab-1', 'a')
    world.quiesce()
    expect(world.violations()).toEqual([])
    expect(world.refreshes.map(({ component }) => component)).toEqual(['b', 'a'])
  })

  it('the opening story answered before any client: refreshed once a client shows it', () => {
    const world = new World()
    world.answer('a') // the dev server's startup prefetch
    world.quiesce()
    expect(world.refreshes).toEqual([])
    world.select('tab-1', 'a')
    world.quiesce()
    expect(world.violations()).toEqual([])
    expect(world.refreshes).toHaveLength(1)
  })

  it('subcomponents pending with the parent: one refresh shows them all', () => {
    const world = new World()
    world.select('tab-1', 'c')
    world.quiesce()
    expect(world.violations()).toEqual([])
    expect(world.refreshes).toHaveLength(1)
    expect(world.stored.get('c')!.checker).toEqual(new Set(['C', 'C.Part']))
  })

  it('a key the refresh finds pending for the first time is watched, and a stopped checker is asked once more', () => {
    const world = new World()
    world.select('tab-1', 'c')
    world.run(0)
    world.complete(0, true) // C answers
    world.complete(0, false) // C.Part's checker stops: not kept
    world.quiesce()
    expect(world.violations()).toEqual([])
    expect(world.stored.get('c')!.checker).toEqual(new Set(['C', 'C.Part']))
    expect(world.watchCount.get('C.Part@0')).toBe(2)
  })

  it('a checker that never keeps an answer does not loop: two watches, then the structural props stand', () => {
    const world = new World()
    world.select('tab-1', 'b')
    for (let round = 0; round < 6; round += 1) {
      while (world.tasks.length > 0) world.run(0)
      while (world.inflight.length > 0) world.complete(0, false)
    }
    expect(world.watchCount.get('B@0')).toBe(2)
    expect(world.refreshes.length).toBeLessThanOrEqual(2)
    expect(world.violations()).toEqual([])
  })
})
