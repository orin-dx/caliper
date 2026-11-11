// The two methods' answers made into one reference, and a tool's prop type scored against it. Pure: nothing here talks
// to the checker or reads a file.
//
// Method A is the checker's type (src/oracle-checker.ts), method B the type the declaration writes
// (src/reference-declared.ts). They are held to the existing comparison (src/oracle-compare.ts), B as the "tool" and A as
// the "checker": identical or spelled is agreement, B looser than A is B saying less (so A stands alone), and B
// contradicting A is a contest a person settles.
import { compare, type Norm } from './oracle-compare.ts'
import { isAgreement, type Outcome } from './vocabulary.ts'

/** Which methods stand behind a reference. */
export type Basis = 'both' | 'checker' | 'declaration'

export type Reference =
  // `why` says why one method stands alone: the other abstained, or wrote a looser type than the checker's.
  | { kind: 'agreed'; basis: Basis; norm: Norm; why?: 'abstained' | 'looser' }
  | { kind: 'contested'; checker: Norm; declared: Norm }
  | { kind: 'none' }

export interface Scored {
  outcome: Outcome
  /** For `correct`, `less precise` and `wrong`: which methods stand behind the verdict. */
  basis?: Basis
  /** For `contested`: the method the tool agrees with, when it agrees with exactly one. */
  leans?: 'checker' | 'declaration'
}

/** A declared type with every name it mentions given itself as its alias, which is how `compare` takes a name for the
 * type it stands for. Where the declaration stands alone the checker could not resolve the name (an unresolved import),
 * so a tool that gives the same name matches it exactly; `compare` would read it as less precise. */
function named(n: Norm): Norm {
  switch (n.k) {
    case 'name': {
      return { ...n, alias: n.name }
    }
    case 'union':
    case 'intersection': {
      return { ...n, members: n.members.map((m) => named(m)) }
    }
    case 'array': {
      return { ...n, element: named(n.element) }
    }
    case 'tuple': {
      return { ...n, elements: n.elements.map((e) => named(e)) }
    }
    case 'function': {
      return {
        ...n,
        params: n.params.map((p) => ({ ...p, type: named(p.type) })),
        returns: named(n.returns),
      }
    }
    default: {
      return n
    }
  }
}

/** Whether a type mentions a name, which is a type this side cannot see into. */
function mentionsName(n: Norm): boolean {
  switch (n.k) {
    case 'name': {
      return true
    }
    case 'union':
    case 'intersection': {
      return n.members.some((m) => mentionsName(m))
    }
    case 'array': {
      return mentionsName(n.element)
    }
    case 'tuple': {
      return n.elements.some((e) => mentionsName(e))
    }
    case 'function': {
      return n.params.some((p) => mentionsName(p.type)) || mentionsName(n.returns)
    }
    default: {
      return false
    }
  }
}

/** A type the checker has nothing to say with: an error, `any`, a top-level `unknown`, or a type it did not evaluate. */
const uninformative = (n: Norm) =>
  n.k === 'error' || n.k === 'opaque' || (n.k === 'prim' && (n.name === 'any' || n.name === 'unknown'))

/** One reference from method A (always has a type) and method B (absent when it abstained). */
export function reference(checker: Norm, declared: Norm | undefined): Reference {
  const hasA = !uninformative(checker)
  if (declared === undefined) {
    return hasA ? { kind: 'agreed', basis: 'checker', norm: checker, why: 'abstained' } : { kind: 'none' }
  }
  if (!hasA) return { kind: 'agreed', basis: 'declaration', norm: named(declared), why: 'abstained' }
  const verdict = compare(declared, checker)
  // Part of the checker's type was not evaluated, so only the declaration says what it is.
  if (verdict === 'not judged') return { kind: 'agreed', basis: 'declaration', norm: named(declared), why: 'abstained' }
  if (isAgreement(verdict)) return { kind: 'agreed', basis: 'both', norm: checker }
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
    const vsChecker = isAgreement(compare(tool, ref.checker))
    const vsDeclared = isAgreement(compare(tool, ref.declared))
    if (vsChecker === vsDeclared) return { outcome: 'contested', leans: undefined }
    return { outcome: 'contested', leans: vsChecker ? 'checker' : 'declaration' }
  }
  const verdict = compare(tool, ref.norm)
  if (verdict === 'checker any/error' || verdict === 'not judged') return { outcome: 'no reference' }
  if (isAgreement(verdict)) return { outcome: 'correct', basis: ref.basis }
  // A declaration standing alone names types it cannot see into, so a mismatch with one is not proof of a wrong type.
  const blind = ref.basis === 'declaration' && mentionsName(ref.norm)
  return { outcome: verdict === 'less precise' || blind ? 'less precise' : 'wrong', basis: ref.basis }
}
