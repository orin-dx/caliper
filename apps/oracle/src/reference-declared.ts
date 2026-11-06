// Method B of the independent type reference (docs/measurement.md, "Independent reference"): the type a prop's
// declaration WRITES, read from source text with TypeScript's parser, never from the checker's type printer.
//
// The checker's answer (method A, src/oracle-checker.ts) and this one fail differently. A can be wrong or empty where
// the checker has no type (an unresolved import, `any`); B cannot see through a type the declaration leaves to be
// computed (`Pick<X, 'a'>`, `T[K]`, a conditional type, an inferred prop), where it says nothing. Where both speak and
// agree, the reference is sound; where both speak and disagree, a person looks.
import ts from 'typescript'
import type { Norm } from './oracle-compare.ts'
import { typeFromText } from './tool-norm.ts'

/** Why method B has no type for a declaration. */
export type Abstention =
  | 'no declaration'
  | 'unsupported declaration'
  | 'no annotation'
  | 'unreadable'
  | 'multiple declarations'

export interface Declared {
  /** What the declaration writes, as text; absent when B abstains. */
  text?: string
  norm?: Norm
  abstained?: Abstention
}

/** The member `declaration` is, parsed inside an interface, or inside a class when that does not parse. */
function memberOf(declaration: string): { source: ts.SourceFile; member: ts.Node } | undefined {
  for (const wrapper of ['interface __I', 'class __C']) {
    const source = ts.createSourceFile(
      'declaration.ts',
      `${wrapper} { ${declaration} }`,
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TS,
    )
    const diagnostics = (source as unknown as { parseDiagnostics: unknown[] }).parseDiagnostics
    const [container] = source.statements
    if (diagnostics.length > 0 || source.statements.length !== 1) continue
    if (!ts.isInterfaceDeclaration(container) && !ts.isClassDeclaration(container)) continue
    if (container.members.length === 1) return { source, member: container.members[0] }
  }
  return undefined
}

/** The type annotation a property or method declaration writes, as text. A method `f(a: A): R` is the function type
 * `(a: A) => R`. */
export function annotationText(declaration: string): { text?: string; abstained?: Abstention } {
  const parsed = memberOf(declaration)
  if (!parsed) return { abstained: 'unsupported declaration' }
  const { source, member } = parsed
  if (ts.isPropertySignature(member) || ts.isPropertyDeclaration(member)) {
    return member.type ? { text: member.type.getText(source) } : { abstained: 'no annotation' }
  }
  if (ts.isMethodSignature(member) || ts.isMethodDeclaration(member)) {
    // With no return type a method is `any` (a signature) or inferred (a declaration with a body): never `void`.
    if (!member.type) return { abstained: 'no annotation' }
    const params = member.parameters.map((p) => p.getText(source)).join(', ')
    return { text: `(${params}) => ${member.type.getText(source)}` }
  }
  return { abstained: 'unsupported declaration' }
}

/** What one declaration, given as its source text, says the prop's type is. A type the declaration leaves to be computed
 * (`opaque`) is an abstention, not a type. */
export function declaredType(declaration: string | undefined): Declared {
  if (declaration === undefined) return { abstained: 'no declaration' }
  const { text, abstained } = annotationText(declaration)
  if (text === undefined) return { abstained }
  const norm = typeFromText(text)
  return norm.k === 'opaque' ? { text, abstained: 'unreadable' } : { text, norm }
}

/** The declarations of one prop (merged interfaces, an intersection's two sources): one answer when they all write the
 * same type, else an abstention, because the checker's type is some combination of them that this reader cannot make. */
export function declaredTypeOfAll(declarations: string[]): Declared {
  if (declarations.length === 0) return { abstained: 'no declaration' }
  const all = declarations.map((d) => declaredType(d))
  const [first] = all
  return all.every((d) => d.text === first.text) ? first : { abstained: 'multiple declarations' }
}
