// The two methods' answers made into one reference, and a tool's prop type scored against it. Pure: nothing here talks
// to the checker or reads a file.
//
// Method A is the checker's type (src/oracle-checker.ts), method B the type the declaration writes
// (src/reference-declared.ts). They are held to the existing comparison (src/oracle-compare.ts), B as the "tool" and A as
// the "checker": identical or spelled is agreement, B looser than A is B saying less (so A stands alone), and B
// contradicting A is a contest a person settles.
import { compare, type Norm, type Verdict } from './oracle-compare.ts'

/** Which methods stand behind a reference. */
export type Basis = 'both' | 'checker' | 'declaration'

export type Reference =
  // `why` says why one method stands alone: the other abstained, or wrote a looser type than the checker's.
  | { kind: 'agreed'; basis: Basis; norm: Norm; why?: 'abstained' | 'looser' }
  | { kind: 'contested'; checker: Norm; declared: Norm }
  | { kind: 'none' }

/** What a prop's reference looks like to a reader, apart from the types themselves. */
export type Outcome = 'correct' | 'less precise' | 'wrong' | 'contested' | 'no reference' | 'layer' | 'missing'

export const OUTCOMES: Outcome[] = ['correct', 'less precise', 'wrong', 'contested', 'no reference', 'layer', 'missing']

export interface Scored {
  outcome: Outcome
  /** For `correct`, `less precise` and `wrong`: which methods stand behind the verdict. */
  basis?: Basis
  /** For `contested`: the method the tool agrees with, when it agrees with exactly one. */
  leans?: 'checker' | 'declaration'
}

/** A type the checker has nothing to say with: an error, `any`, or a top-level `unknown`. */
const uninformative = (n: Norm) => n.k === 'error' || (n.k === 'prim' && (n.name === 'any' || n.name === 'unknown'))

const AGREE = new Set<Verdict>(['identical', 'spelled'])
const agrees = (verdict: Verdict) => AGREE.has(verdict)

/** One reference from method A (always has a type) and method B (absent when it abstained). */
export function reference(checker: Norm, declared: Norm | undefined): Reference {
  const hasA = !uninformative(checker)
  if (declared === undefined) {
    return hasA ? { kind: 'agreed', basis: 'checker', norm: checker, why: 'abstained' } : { kind: 'none' }
  }
  if (!hasA) return { kind: 'agreed', basis: 'declaration', norm: declared, why: 'abstained' }
  const verdict = compare(declared, checker)
  if (agrees(verdict)) return { kind: 'agreed', basis: 'both', norm: checker }
  // The declaration is looser than the checker's type (a name or an unbound parameter the checker evaluated): B said less.
  if (verdict === 'less precise') return { kind: 'agreed', basis: 'checker', norm: checker, why: 'looser' }
  return { kind: 'contested', checker, declared }
}

/** One tool prop type against a reference. `tool` is absent when the tool does not list the prop; `layered` says a
 * collapsed layer's member list names it, which surfaces the prop without a type to judge. */
export function score(tool: Norm | undefined, ref: Reference, layered: boolean): Scored {
  if (tool === undefined) return { outcome: layered ? 'layer' : 'missing' }
  if (ref.kind === 'none') return { outcome: 'no reference' }
  if (ref.kind === 'contested') {
    const vsChecker = agrees(compare(tool, ref.checker))
    const vsDeclared = agrees(compare(tool, ref.declared))
    if (vsChecker === vsDeclared) return { outcome: 'contested', leans: undefined }
    return { outcome: 'contested', leans: vsChecker ? 'checker' : 'declaration' }
  }
  const verdict = compare(tool, ref.norm)
  if (verdict === 'checker any/error') return { outcome: 'no reference' }
  if (agrees(verdict)) return { outcome: 'correct', basis: ref.basis }
  return { outcome: verdict === 'less precise' ? 'less precise' : 'wrong', basis: ref.basis }
}
