// Scores one tool (Caliper, react-docgen-typescript or react-docgen) against the checker over the same universe of props,
// so the three are held to the same rules. Pure: nothing here talks to the checker or reads a file.
//
// The universe is every own prop the checker finds on a component export (React's inherited DOM attributes, `ref` and
// `key` are not part of it; src/oracle.ts filters them). A tool's prop is `listed` when the tool reports it, and gets
// the same verdict the oracle gives Caliper (src/oracle-compare.ts); otherwise it is `missing`, for one of three
// reasons, and counted by where the checker says the prop is declared.
import { compare, VERDICTS, type Norm, type Verdict } from './oracle-compare.ts'

/** Where a prop the tool does not list is declared, by the checker's first declaration. */
export const STRATA = ['same_file', 'other_project_file', 'dependency', 'unknown'] as const
export type Stratum = (typeof STRATA)[number]

/** Why a tool does not list a prop: it reports no such component, the tool threw on the component's file, it reports
 * the component but not that prop, or it lists the prop only as a member of an inherited layer it collapsed
 * (Caliper's `inheritance`, which is a deliberate choice, not an omission). */
export const REASONS = ['component', 'failed_file', 'prop', 'collapsed'] as const
export type Reason = (typeof REASONS)[number]

/** One prop the checker finds, with the checker's type already reduced to a `Norm`. */
export interface UniverseProp {
  /** The component, as the tools are matched to it: file and normalised name. */
  component: string
  prop: string
  stratum: Stratum
  checker: Norm
}

/** What a tool reports for a universe prop: its type as a `Norm`, or why it has none. */
export type Reported = Norm | Reason

export interface ToolScore {
  /** Props the checker finds on the components of the universe. */
  universe: number
  listed: number
  missing: number
  verdicts: Record<Verdict, number>
  /** Identical or spelled, of the whole universe: a prop the tool omits counts against it. */
  agree_universe: number
  /** Identical or spelled, of the props the tool lists. */
  agree_listed: number
  /** Wrong, of the props the tool lists. */
  wrong_listed: number
  missing_by_stratum: Record<Stratum, number>
  missing_by_reason: Record<Reason, number>
  /** Components with at least one prop in the universe, and how many of them the tool reports at all. */
  components: number
  components_found: number
  /** The first few props of each kind the tool gets wrong or less precise, to review the adapters by. */
  examples: Record<'wrong' | 'less precise', { component: string; prop: string; tool: Norm; checker: Norm }[]>
}

const EXAMPLES = 8

const isReason = (value: Reported): value is Reason => typeof value === 'string'
const rate = (n: number, of: number) => (of === 0 ? 0 : n / of)

/** `pick` says what the tool reports for one universe prop. */
export function scoreTool<T extends UniverseProp>(universe: readonly T[], pick: (u: T) => Reported): ToolScore {
  const verdicts = Object.fromEntries(VERDICTS.map((v) => [v, 0])) as Record<Verdict, number>
  const byStratum = Object.fromEntries(STRATA.map((s) => [s, 0])) as Record<Stratum, number>
  const byReason = Object.fromEntries(REASONS.map((r) => [r, 0])) as Record<Reason, number>
  const components = new Set<string>()
  const found = new Set<string>()
  const examples: ToolScore['examples'] = { wrong: [], 'less precise': [] }
  let listed = 0
  for (const u of universe) {
    components.add(u.component)
    const reported = pick(u)
    // A component is found when any of its props is not missing for want of the component itself.
    if (!isReason(reported) || (reported !== 'component' && reported !== 'failed_file')) found.add(u.component)
    if (isReason(reported)) {
      byReason[reported]++
      // A collapsed layer is a choice, not an omission, so it has no stratum.
      if (reported !== 'collapsed') byStratum[u.stratum]++
      continue
    }
    listed++
    const verdict = compare(reported, u.checker)
    verdicts[verdict]++
    if ((verdict === 'wrong' || verdict === 'less precise') && examples[verdict].length < EXAMPLES) {
      examples[verdict].push({
        component: u.component.replace('\0', ' '),
        prop: u.prop,
        tool: reported,
        checker: u.checker,
      })
    }
  }
  const agree = verdicts.identical + verdicts.spelled
  return {
    universe: universe.length,
    listed,
    missing: universe.length - listed,
    verdicts,
    agree_universe: rate(agree, universe.length),
    agree_listed: rate(agree, listed),
    wrong_listed: rate(verdicts.wrong, listed),
    missing_by_stratum: byStratum,
    missing_by_reason: byReason,
    components: components.size,
    components_found: found.size,
    examples,
  }
}

/** Like `scoreTool`, over the components the tool reports only: a prop of a component the tool does not report at all
 * (`component`) is left out, so a function the checker takes for a component and the tool rightly ignores does not
 * count against it. This is the universe of the weekly headline: what a reader of the tool's output is shown. */
export function scoreReported<T extends UniverseProp>(universe: readonly T[], pick: (u: T) => Reported): ToolScore {
  return scoreTool(
    universe.filter((u) => pick(u) !== 'component'),
    pick,
  )
}
