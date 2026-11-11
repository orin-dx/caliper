// What the independent reference reports: one row per judged prop, counted per corpus, and the contested props written
// out for a person to settle. Pure.
import type { Abstention } from './reference-declared.ts'
import type { Basis } from './reference-judge.ts'
import { OUTCOMES, type Outcome } from './vocabulary.ts'
import type { Origin, Stratum } from './reference-sample.ts'

export interface Row {
  corpus: string
  stratum: Stratum
  /** Where the prop is declared: the component's own file, another project file, a dependency, or `@types/react`. */
  origin: Origin
  file: string
  component: string
  prop: string
  /** The checker's type as it prints it, for a reader (the judging uses the structured form). */
  checker: string
  /** What the declaration writes; absent when method B abstained. */
  declared?: string
  abstained?: Abstention
  reference: 'agreed' | 'contested' | 'none'
  basis?: Basis
  why?: 'abstained' | 'looser'
  /** For a contested prop: the kinds of the two types, `declared->checker`. */
  pair?: string
  /** Where the declaration is. */
  location?: string
  /** The tool's score; absent when no tool output was given or the tool has no single component of that name. */
  tool?: { outcome: Outcome; basis?: Basis; leans?: 'checker' | 'declaration' }
}

const count = <T extends string>(values: T[]) => {
  const out: Record<string, number> = {}
  for (const v of values) out[v] = (out[v] ?? 0) + 1
  return out
}

/** The contested rows most worth reading: those the tool settles (it agrees with exactly one method) first, then the
 * rest the tool has a type for, then the others. */
export function informative(rows: Row[], limit = 10): Row[] {
  const rank = (r: Row) => {
    if (r.tool?.leans) return 0
    return r.tool ? 1 : 2
  }
  return rows
    .filter((r) => r.reference === 'contested')
    .toSorted((a, b) => rank(a) - rank(b))
    .slice(0, limit)
}

/** One corpus's counts. `withTool` is whether a tool's output was given, so a prop with no score is one the tool has no
 * single component of that name for (`tool_unmatched`), not one nobody tried to score. */
export function summarize(rows: Row[], withTool: boolean) {
  const scored = rows.filter((r) => r.tool)
  const scoredStrata = count(scored.map((r) => r.stratum))
  const outcomesByStratum = Object.keys(scoredStrata).map((stratum) => {
    const outcomes = scored.filter((r) => r.stratum === stratum).map((r) => r.tool!.outcome)
    return [stratum, count(outcomes)]
  })
  const scoredOrigins = count(scored.map((r) => r.origin))
  const outcomesByOrigin = Object.keys(scoredOrigins).map((origin) => {
    const outcomes = scored.filter((r) => r.origin === origin).map((r) => r.tool!.outcome)
    return [origin, count(outcomes)]
  })
  return {
    sampled: rows.length,
    by_stratum: count(rows.map((r) => r.stratum)),
    reference: {
      agreed_both: rows.filter((r) => r.basis === 'both').length,
      checker_only: rows.filter((r) => r.basis === 'checker').length,
      declaration_only: rows.filter((r) => r.basis === 'declaration').length,
      contested: rows.filter((r) => r.reference === 'contested').length,
      none: rows.filter((r) => r.reference === 'none').length,
    },
    declaration_abstained: count(rows.flatMap((r) => (r.abstained ? [r.abstained] : []))),
    // Only a reference the checker stood behind alone; where the declaration did, `why` says the checker abstained.
    checker_alone_because: count(rows.flatMap((r) => (r.why && r.basis === 'checker' ? [r.why] : []))),
    contested_kinds: count(rows.flatMap((r) => (r.pair ? [r.pair] : []))),
    scored: {
      props: scored.length,
      tool_unmatched: withTool ? rows.length - scored.length : undefined,
      outcomes: Object.fromEntries(OUTCOMES.map((o) => [o, scored.filter((r) => r.tool?.outcome === o).length])),
      correct_by_basis: count(scored.flatMap((r) => (r.tool?.outcome === 'correct' ? [r.tool.basis!] : []))),
      wrong_by_basis: count(scored.flatMap((r) => (r.tool?.outcome === 'wrong' ? [r.tool.basis!] : []))),
      contested_leans: count(scored.flatMap((r) => (r.tool?.leans ? [r.tool.leans] : []))),
      by_stratum: Object.fromEntries(outcomesByStratum),
      by_origin: Object.fromEntries(outcomesByOrigin),
    },
    informative: informative(rows),
  }
}

/** The contested props as text for a person: both types, where the declaration is, and what the tool said. */
export function reviewText(rows: Row[]): string {
  const contested = rows.filter((r) => r.reference === 'contested')
  const lines = [
    `# Contested props (${contested.length})`,
    '',
    'The checker (A) and the declaration (B) disagree. Decide which is the type a reader should see.',
    '',
  ]
  for (const r of contested) {
    lines.push(
      `## ${r.corpus}: ${r.component}.${r.prop}`,
      '',
      `- file: ${r.file}`,
      `- declaration: ${r.location ?? 'unknown'}`,
      `- checker (A): ${r.checker}`,
      `- declared (B): ${r.declared ?? 'none'}`,
      `- tool: ${r.tool ? `${r.tool.outcome}${r.tool.leans ? `, agrees with ${r.tool.leans}` : ''}` : 'not scored'}`,
      '',
    )
  }
  return lines.join('\n')
}
