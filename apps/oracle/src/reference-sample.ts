// Which props the independent reference judges: every prop a list names, plus a seeded, stratified control sample, so
// the control sample is not all `string` and `boolean`. Pure: nothing here talks to the checker.

/** Where a prop's declaration sits relative to the component. */
export type Origin = 'own' | 'extends' | 'dependency' | 'html'
/** How the component's props type is built. */
export type PropsShape = 'plain' | 'intersection' | 'mapped'
/** What the prop's type is. */
export type ValueKind = 'generic' | 'function' | 'union_literal' | 'other'

/** One prop of one exported component, with what is known without judging its type. */
export interface Candidate {
  corpus: string
  /** Path of the component's file, relative to the corpus root. */
  file: string
  component: string
  prop: string
  origin: Origin
  shape: PropsShape
  kind: ValueKind
}

/** The strata a prop falls in. `stratumOf` gives it the first it fits, in this order, so the harder shapes are not
 * swamped by the common ones. */
export type Stratum =
  | 'listed'
  | 'generic'
  | 'mapped'
  | 'intersection'
  | 'function'
  | 'union_literal'
  | 'dependency'
  | 'html'
  | 'extends'
  | 'own'

export function stratumOf(c: Candidate, listed: boolean): Stratum {
  if (listed) return 'listed'
  if (c.kind === 'generic') return 'generic'
  if (c.shape === 'mapped') return 'mapped'
  if (c.shape === 'intersection') return 'intersection'
  if (c.kind === 'function') return 'function'
  if (c.kind === 'union_literal') return 'union_literal'
  if (c.origin === 'dependency') return 'dependency'
  if (c.origin === 'html') return 'html'
  return c.origin === 'extends' ? 'extends' : 'own'
}

/** A small seeded generator (mulberry32), so a sample is the same on every run and machine. */
export function seeded(seed: string): () => number {
  let state = 0
  for (const ch of seed) state = Math.imul(state ^ ch.codePointAt(0)!, 2_654_435_761) >>> 0
  return () => {
    state = (state + 1_831_565_813) >>> 0
    let t = state
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4_294_967_296
  }
}

/** `items` in a seeded order. */
export function shuffled<T>(items: T[], random: () => number): T[] {
  const out = [...items]
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1))
    ;[out[i], out[j]] = [out[j], out[i]]
  }
  return out
}

export const keyOf = (c: Pick<Candidate, 'file' | 'component' | 'prop'>) =>
  `${c.file}\u0000${c.component}\u0000${c.prop}`

/** The control sample: `target` candidates spread over the strata the pool has, the smaller strata whole and the
 * remainder shared out evenly. `pool` is sorted before it is shuffled, so the order the checker visits files in does not
 * change the sample. */
export function stratified(pool: Candidate[], target: number, seed: string): Candidate[] {
  const random = seeded(seed)
  const sorted = pool.toSorted((a, b) => keyOf(a).localeCompare(keyOf(b)))
  const groups = new Map<Stratum, Candidate[]>()
  for (const c of sorted) {
    const stratum = stratumOf(c, false)
    groups.set(stratum, [...(groups.get(stratum) ?? []), c])
  }
  const order = [...groups.entries()].toSorted((a, b) => a[1].length - b[1].length)
  const chosen: Candidate[] = []
  let remaining = Math.min(target, sorted.length)
  for (const [i, [, members]] of order.entries()) {
    const take = Math.min(members.length, Math.ceil(remaining / (order.length - i)))
    chosen.push(...shuffled(members, random).slice(0, take))
    remaining -= take
  }
  return chosen
}
