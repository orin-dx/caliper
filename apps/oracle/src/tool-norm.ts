// The other tools' side of the oracle: react-docgen-typescript (RDT) and react-docgen prop types reduced to the `Norm`
// shape src/oracle-compare.ts compares, so all three tools are held to the checker by the same rules. Pure: nothing
// here talks to the checker.
//
// Both tools state a type as source text (RDT prints it, react-docgen keeps what was written), so there is one mapper,
// `typeFromText`, which reads the text with TypeScript's parser. What a tool leaves as a name stays a name, and what it
// cannot state (a truncated `Foo<...>`, an indexed access, a conditional type) is `opaque`, which the comparison reads
// as less precise, never wrong.
import ts from 'typescript'
import { bareName, MAX_DEPTH, prim, type Norm } from './oracle-compare.ts'

const OPAQUE: Norm = { k: 'opaque' }

const KEYWORDS = new Map<ts.SyntaxKind, string>([
  [ts.SyntaxKind.StringKeyword, 'string'],
  [ts.SyntaxKind.NumberKeyword, 'number'],
  [ts.SyntaxKind.BooleanKeyword, 'boolean'],
  [ts.SyntaxKind.BigIntKeyword, 'bigint'],
  [ts.SyntaxKind.SymbolKeyword, 'symbol'],
  [ts.SyntaxKind.UndefinedKeyword, 'undefined'],
  [ts.SyntaxKind.AnyKeyword, 'any'],
  [ts.SyntaxKind.UnknownKeyword, 'unknown'],
  [ts.SyntaxKind.NeverKeyword, 'never'],
  [ts.SyntaxKind.VoidKeyword, 'void'],
  [ts.SyntaxKind.ObjectKeyword, 'object'],
])
const ARRAYS = new Set(['Array', 'ReadonlyArray'])

/** The name a member is declared with, when it is written as a plain name or string. */
const memberName = (name: ts.PropertyName | undefined) =>
  name && (ts.isIdentifier(name) || ts.isStringLiteral(name) || ts.isNumericLiteral(name)) ? name.text : undefined

function literal(node: ts.LiteralTypeNode['literal']): Norm {
  switch (node.kind) {
    case ts.SyntaxKind.NullKeyword: {
      return prim('null')
    }
    case ts.SyntaxKind.TrueKeyword:
    case ts.SyntaxKind.FalseKeyword: {
      return { k: 'lit', value: node.kind === ts.SyntaxKind.TrueKeyword ? 'true' : 'false' }
    }
    case ts.SyntaxKind.NumericLiteral: {
      // The checker's literal value is the number, so `1.0` is `1`.
      return { k: 'lit', value: String(Number(node.text)) }
    }
    case ts.SyntaxKind.PrefixUnaryExpression: {
      const { operand } = node as ts.PrefixUnaryExpression
      return ts.isNumericLiteral(operand) ? { k: 'lit', value: `-${Number(operand.text)}` } : OPAQUE
    }
    case ts.SyntaxKind.StringLiteral:
    case ts.SyntaxKind.NoSubstitutionTemplateLiteral: {
      return { k: 'lit', value: (node as ts.StringLiteral).text }
    }
    case ts.SyntaxKind.BigIntLiteral: {
      return { k: 'lit', value: (node as ts.BigIntLiteral).text.replace(/n$/, '') }
    }
    default: {
      return OPAQUE
    }
  }
}

function convert(node: ts.TypeNode, depth: number): Norm {
  if (depth > MAX_DEPTH) return OPAQUE
  const inner = (n: ts.TypeNode) => convert(n, depth + 1)
  const keyword = KEYWORDS.get(node.kind)
  if (keyword) return prim(keyword)
  if (ts.isParenthesizedTypeNode(node)) return convert(node.type, depth)
  if (ts.isLiteralTypeNode(node)) return literal(node.literal)
  if (ts.isTemplateLiteralTypeNode(node)) return prim('string')
  if (ts.isUnionTypeNode(node)) return { k: 'union', members: node.types.map((n) => inner(n)) }
  if (ts.isIntersectionTypeNode(node)) {
    const parts = node.types.map((n) => inner(n))
    // `string & {}` is a string, as the checker's side reads it.
    const meaningful = parts.filter((m) => !(m.k === 'object' && m.fields.length === 0 && !m.index))
    return meaningful.length === 1 ? meaningful[0] : { k: 'intersection', members: parts }
  }
  if (ts.isArrayTypeNode(node)) return { k: 'array', element: inner(node.elementType) }
  if (ts.isTupleTypeNode(node)) {
    // A named, optional or rest element is the type it wraps.
    const element = (n: ts.TypeNode): Norm => {
      if (ts.isNamedTupleMember(n) || ts.isOptionalTypeNode(n) || ts.isRestTypeNode(n)) return inner(n.type)
      return inner(n)
    }
    return { k: 'tuple', elements: node.elements.map((n) => element(n)) }
  }
  if (ts.isFunctionTypeNode(node)) {
    const params = node.parameters.map((p) => ({
      optional: Boolean(p.questionToken ?? p.initializer),
      rest: Boolean(p.dotDotDotToken),
      type: p.type ? inner(p.type) : OPAQUE,
    }))
    return { k: 'function', params, returns: inner(node.type) }
  }
  if (ts.isTypeLiteralNode(node)) {
    const [only] = node.members
    if (node.members.length === 1 && ts.isCallSignatureDeclaration(only)) {
      const params = only.parameters.map((p) => ({
        optional: Boolean(p.questionToken ?? p.initializer),
        rest: Boolean(p.dotDotDotToken),
        type: p.type ? inner(p.type) : OPAQUE,
      }))
      return { k: 'function', params, returns: only.type ? inner(only.type) : OPAQUE }
    }
    // A member with no type (`{string}`) is not something either tool states: RDT prints a type it could not name that way.
    if (node.members.some((m) => ts.isPropertySignature(m) && !m.type)) return OPAQUE
    const fields = node.members.flatMap((m) => {
      const name = ts.isPropertySignature(m) || ts.isMethodSignature(m) ? memberName(m.name) : undefined
      return name === undefined ? [] : [name]
    })
    return {
      k: 'object',
      fields: fields.toSorted(),
      index: node.members.some((m) => ts.isIndexSignatureDeclaration(m)),
    }
  }
  if (ts.isTypeOperatorNode(node)) {
    // `readonly string[]` is the array; `keyof X` and `unique symbol` are not something both sides state.
    return node.operator === ts.SyntaxKind.ReadonlyKeyword ? convert(node.type, depth) : OPAQUE
  }
  if (ts.isTypeReferenceNode(node)) {
    const name = bareName(node.typeName.getText())
    if (ARRAYS.has(name) && node.typeArguments?.length === 1)
      return { k: 'array', element: inner(node.typeArguments[0]) }
    return { k: 'name', name }
  }
  if (ts.isImportTypeNode(node) && node.qualifier) return { k: 'name', name: bareName(node.qualifier.getText()) }
  // Indexed access, conditional, mapped, `typeof X`, `infer`, `this`: a tool that left one of these as written has not
  // evaluated it.
  return OPAQUE
}

/** One TypeScript type, as written or as a tool printed it, reduced to a `Norm`. Text that does not parse as a type (a
 * printer's `...` for a type it cut short) is `opaque`. */
export function typeFromText(text: string): Norm {
  if (text.trim() === '') return OPAQUE
  // Parents are set so a node can print its own name. `parseDiagnostics` is not in the public typings.
  const source = ts.createSourceFile('type.ts', `type T = ${text}`, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
  const diagnostics = (source as unknown as { parseDiagnostics: unknown[] }).parseDiagnostics
  const [statement] = source.statements
  if (diagnostics.length > 0 || source.statements.length !== 1 || !ts.isTypeAliasDeclaration(statement)) return OPAQUE
  return convert(statement.type, 0)
}

/** What react-docgen-typescript reports for a prop's type (`prop.type`): the printed text as `name`, or `enum` with the
 * literal `value`s when `shouldExtractLiteralValuesFromEnum` is on and the type is a literal union. */
export interface RdtType {
  name: string
  raw?: string
  value?: { value: string }[]
}

/** An RDT prop type. A literal union is the union of its values; the `raw` name (`SizeType`) is the alias RDT expanded. */
export function rdtNorm(type: RdtType | undefined): Norm {
  if (!type) return OPAQUE
  if (type.name === 'enum') {
    if (!type.value?.length) return OPAQUE
    const alias = type.raw && /^[A-Za-z_$][\w$]*$/.test(type.raw) ? type.raw : undefined
    return { k: 'union', members: type.value.map((v) => typeFromText(v.value)), alias }
  }
  return typeFromText(type.name)
}

/** What react-docgen reports for a prop's type (`prop.tsType`), without `signature`. */
export interface RdgType {
  name: string
  raw?: string
  value?: string
  elements?: RdgType[]
}

// A `name` that is a kind of type react-docgen reports with a source text, not a type of that name.
const KINDS = new Set(['union', 'intersection', 'signature', 'tuple', 'Array', 'literal'])

/** A react-docgen prop type: the source text it kept where it kept one (`raw`), else what its `name` and `elements` say. */
export function rdgNorm(type: RdgType | undefined): Norm {
  if (!type) return OPAQUE
  if (type.name === 'literal') return type.value === undefined ? OPAQUE : typeFromText(type.value)
  if (type.raw) return typeFromText(type.raw)
  const members = (type.elements ?? []).map((e) => rdgNorm(e))
  if (type.name === 'union' && members.length > 0) return { k: 'union', members }
  if (type.name === 'intersection' && members.length > 0) return { k: 'intersection', members }
  if (type.name === 'Array' && members.length === 1) return { k: 'array', element: members[0] }
  if (type.name === 'tuple' && members.length > 0) return { k: 'tuple', elements: members }
  return KINDS.has(type.name) ? OPAQUE : typeFromText(type.name)
}
