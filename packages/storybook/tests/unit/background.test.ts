// A model-based test of what the provider asked the checker for ahead of the client (`prefetchStep` in
// `src/background.ts`): a simulated provider and checker queue drive the reducer through random event sequences, then
// run to quiescence, and the checker's queue is compared with what the client may open next.
import { describe, expect, it } from 'vitest'
import { initialPrefetched, type PrefetchEffect, type PrefetchEvent, prefetchStep } from '../../src/background.js'

const KEYS = ['A', 'B', 'C', 'D', 'E']

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

/** One ask the provider sent, whose `asked` event has not reached the reducer yet. */
interface InFlight {
  generation: number
  keys: string[]
  revision: number
}

/** The provider, the binding's queue and the files, with the binding's rules: an edit drops every queued request. */
class World {
  readonly state = initialPrefetched()
  /** The keys of the current list's stories, which the client may open next. */
  wanted = new Set<string>()
  /** The checker's queued requests, and the answers it keeps at `revision`. */
  queued = new Set<string>()
  kept = new Set<string>()
  revision = 0
  inFlight: InFlight[] = []

  private apply(effects: PrefetchEffect[]): void {
    for (const effect of effects) {
      if (effect.type === 'cancel') for (const key of effect.keys) this.queued.delete(key)
      else this.ask(effect.keys, this.state.generation)
    }
  }

  private dispatch(event: PrefetchEvent): void {
    this.apply(prefetchStep(this.state, event))
  }

  /** The binding queues `keys` now; the provider reports it later. */
  ask(keys: string[], generation: number): void {
    this.dispatch({ type: 'asking', generation, keys })
    for (const key of keys) if (!this.kept.has(key)) this.queued.add(key)
    this.inFlight.push({ generation, keys, revision: this.revision })
  }

  replace(keys: string[]): void {
    this.wanted = new Set(keys)
    this.dispatch({ type: 'replaced' })
    // The drain asks for each story of the new list.
    for (const key of keys) this.ask([key], this.state.generation)
  }

  /** A story of the current list asked for again (a drain still running, or one the client went back to). */
  again(key: string): void {
    this.wanted.add(key)
    this.ask([key], this.state.generation)
  }

  report(at: number): void {
    const [flight] = this.inFlight.splice(at, 1)
    if (flight) this.dispatch({ type: 'asked', ...flight })
  }

  edit(): void {
    this.revision += 1
    this.queued.clear()
    this.kept.clear()
  }

  seen(): void {
    this.dispatch({ type: 'seen', revision: this.revision })
  }

  answer(key: string): void {
    if (this.queued.delete(key)) this.kept.add(key)
  }

  quiesce(): void {
    this.seen()
    while (this.inFlight.length > 0) this.report(0)
  }
}

describe('the prefetch reducer, over random event sequences', () => {
  it('leaves queued exactly what the current list asked for and the checker has not answered', () => {
    for (let seed = 1; seed <= 400; seed += 1) {
      const pick = generator(seed)
      const world = new World()
      for (let n = 0; n < 40; n += 1) {
        const choice = pick(6)
        if (choice === 0) world.replace(KEYS.filter(() => pick(2) === 0))
        else if (choice === 1 && world.inFlight.length > 0) world.report(pick(world.inFlight.length))
        else if (choice === 2) world.edit()
        else if (choice === 3) world.seen()
        else if (choice === 4) world.again(KEYS[pick(KEYS.length)]!)
        else world.answer(KEYS[pick(KEYS.length)]!)
      }
      world.quiesce()
      const covered = [...world.wanted].filter((key) => world.queued.has(key) || world.kept.has(key))
      expect(covered, `seed ${seed}: every wanted key queued or answered`).toEqual([...world.wanted])
      expect(
        [...world.queued].filter((key) => !world.wanted.has(key)),
        `seed ${seed}: nothing unwanted queued`,
      ).toEqual([])
    }
  })

  it('cancels what a replaced list asked for, and asks again after an edit', () => {
    const state = initialPrefetched()
    expect(prefetchStep(state, { type: 'asking', generation: 0, keys: ['A'] })).toEqual([])
    expect(prefetchStep(state, { type: 'asked', generation: 0, keys: ['A'], revision: 1 })).toEqual([])
    expect(prefetchStep(state, { type: 'seen', revision: 2 })).toEqual([{ type: 'ask', keys: ['A'] }])
    expect(prefetchStep(state, { type: 'seen', revision: 2 })).toEqual([])
    expect(prefetchStep(state, { type: 'replaced' })).toEqual([{ type: 'cancel', keys: ['A'] }])
    // A story of the old list resolved after the replacement: cancelled, unless the new list asked for it too.
    expect(prefetchStep(state, { type: 'asking', generation: 1, keys: ['B'] })).toEqual([])
    expect(prefetchStep(state, { type: 'asked', generation: 0, keys: ['B', 'C'], revision: 2 })).toEqual([
      { type: 'cancel', keys: ['C'] },
    ])
    expect(prefetchStep(state, { type: 'asked', generation: 1, keys: ['B'], revision: 2 })).toEqual([])
    // Asked before an edit the provider already saw: asked again.
    expect(prefetchStep(state, { type: 'seen', revision: 3 })).toEqual([{ type: 'ask', keys: ['B'] }])
    expect(prefetchStep(state, { type: 'asked', generation: 1, keys: ['D'], revision: 2 })).toEqual([
      { type: 'ask', keys: ['D'] },
    ])
  })
})
