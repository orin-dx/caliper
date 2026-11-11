// The words the oracle's results are written in, defined once. `src/vocabulary.json` is this file written as data
// (`pnpm --filter caliper-docgen-oracle vocabulary`), so scripts/weekly.py and scripts/headline.py read the same lists;
// tests/vocabulary.test.ts fails while the two differ, and scripts/test_vocabulary.py checks the Python side against the
// file.

/** What the comparison (src/oracle-compare.ts) says of one prop. `not judged` is a type the checker side stopped short
 * of (cut at the depth limit, or a kind the normalizer does not read): nothing was compared, so it agrees with nothing. */
export const VERDICTS = ['identical', 'spelled', 'less precise', 'wrong', 'checker any/error', 'not judged'] as const
export type Verdict = (typeof VERDICTS)[number]

/** The verdicts that say Caliper's type is the checker's. */
export const AGREE = ['identical', 'spelled'] as const satisfies readonly Verdict[]

/** The verdicts one comparison of parts can come to, best to worst: a whole is as bad as its worst part. A part the
 * checker side did not judge sits between agreement and a difference, so it never reads as agreement. */
export const SEVERITY: readonly Verdict[] = ['identical', 'spelled', 'not judged', 'less precise', 'wrong']

/** What the construct suite says of a row (src/construct-suite.ts), best to worst: the verdicts, plus the two cases the
 * comparison cannot make. `checker any/error` and `not judged` are fixture problems (the golden has no type to hold
 * Caliper to), ranked last so they never read as an improvement. */
export const ROW_VERDICTS = [
  'identical',
  'spelled',
  'opaque with marker',
  'less precise',
  'wrong',
  'missing',
  'checker any/error',
  'not judged',
] as const
export type RowVerdict = (typeof ROW_VERDICTS)[number]

/** What a prop's reference looks like to a reader (src/reference-judge.ts). */
export const OUTCOMES = ['correct', 'less precise', 'wrong', 'contested', 'no reference', 'layer', 'missing'] as const
export type Outcome = (typeof OUTCOMES)[number]

export const vocabulary = {
  verdicts: VERDICTS,
  agree: AGREE,
  severity: SEVERITY,
  rowVerdicts: ROW_VERDICTS,
  outcomes: OUTCOMES,
}

export const isAgreement = (verdict: string): boolean => AGREE.some((agreed) => agreed === verdict)
