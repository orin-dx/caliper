// Caliper's side of src/oracle-compare.ts: its canonical `PropType` JSON reduced to the shape the comparison reads, as
// the checker side (src/oracle-checker.ts) reduces the checker's type.
import {
  bareName,
  isBareObject,
  lit,
  MAX_DEPTH,
  prim,
  type HtmlFamily,
  type LiteralKind,
  type Norm,
} from './oracle-compare.ts'

const TERMINALS: Record<string, string> = {
  reactNode: 'ReactNode',
  cssProperties: 'CSSProperties',
  elementType: 'ElementType',
  ref: 'Ref',
  sxProps: 'SxProps',
}
const PRIMITIVES = new Set(['string', 'number', 'boolean', 'null', 'undefined', 'any', 'unknown', 'never', 'void'])
const LITERAL_KINDS: Record<string, LiteralKind> = {
  stringLiteral: 'string',
  numberLiteral: 'number',
  boolLiteral: 'boolean',
  bigintLiteral: 'bigint',
}
const LITERALS = new Set(Object.keys(LITERAL_KINDS))

const isIndexField = (field: any) => field.indexKey !== undefined

/** A field's name as the checker side keys it. Caliper writes a member keyed by a symbol as the source does, `[tag]` or
 * `[Symbol.iterator]`; the checker keys it `__@tag@<symbol id>`, which src/oracle-checker.ts reads as `__@tag`. */
export function fieldName(name: string): string {
  const symbol = /^\[(?:Symbol\.)?([A-Za-z_$][\w$]*)\]$/.exec(name)
  return symbol ? `__@${symbol[1]}` : name
}

/** An object type's fields. Caliper writes an index signature as one field whose `indexKey` is the key type: its name is
 * the signature's parameter name (`[id: string]: number`), not a property. */
function objectNorm(fields: any[], inner: (m: any) => Norm): Norm {
  const indexed: any[] = fields.filter((field: any) => isIndexField(field))
  const named: any[] = fields.filter((field: any) => !isIndexField(field))
  const indexValues: Norm[] = indexed.map((field) => inner(field.propType))
  return {
    k: 'object',
    fields: named.map((field) => fieldName(field.name)).toSorted(),
    index: indexed.length > 0,
    ...(indexed.length > 0 && {
      indexValue: indexValues.length === 1 ? indexValues[0] : { k: 'union', members: indexValues },
    }),
  }
}

/** The string keys `Omit<X, K>` names: one literal, or a union of them. Undefined for anything else. */
function omittedKeys(key: Norm): string[] | undefined {
  if (key.k === 'lit' && key.of === 'string') return [key.value]
  if (key.k !== 'union') return undefined
  const keys = key.members.map((m) => omittedKeys(m))
  return keys.every((k) => k !== undefined) ? keys.flat() : undefined
}

/** `Omit<HTMLAttributes<…>, "key" | …>`: the family with the keys it leaves out, so the comparison can expand it. */
function omittedFamily(t: any, inner: (m: any) => Norm): HtmlFamily | undefined {
  if (bareName(t.name) !== 'Omit' || t.args?.length !== 2 || t.args[0].kind !== 'htmlAttributes') return undefined
  const keys = omittedKeys(inner(t.args[1]))
  return keys && { element: t.args[0].element, omitted: [...(t.args[0].omitted ?? []), ...keys] }
}

/** Caliper's canonical `PropType` JSON, normalized. */
export function caliperNorm(t: any, depth = 0): Norm {
  if (!t || depth > MAX_DEPTH) return { k: 'opaque' }
  if (PRIMITIVES.has(t.kind)) return prim(t.kind)
  if (LITERALS.has(t.kind)) return lit(String(t.value), LITERAL_KINDS[t.kind])
  const inner = (m: any) => caliperNorm(m, depth + 1)
  switch (t.kind) {
    case 'literalUnion': {
      return { k: 'union', members: t.members.map((m: string) => lit(m, 'string')) }
    }
    case 'union': {
      return { k: 'union', members: t.members.map((m: any) => inner(m)) }
    }
    case 'intersection': {
      const parts: Norm[] = t.members.map((m: any) => inner(m))
      const meaningful = parts.filter((m) => !isBareObject(m))
      return meaningful.length === 1 ? meaningful[0] : { k: 'intersection', members: parts }
    }
    case 'array': {
      return { k: 'array', element: inner(t.element) }
    }
    // An element is `{ type, label?, modifier? }`. The checker side lists a rest element by its element type and has no
    // markers, so neither does this one.
    case 'tuple': {
      return { k: 'tuple', elements: (t.elements ?? []).map((e: any) => inner(e.type)) }
    }
    case 'object': {
      return objectNorm(t.fields, inner)
    }
    // As the checker side reads an object type with signatures: its properties when it has any, else its first call
    // signature as a function, else (construct signatures only) an empty object.
    case 'callable': {
      const call = t.signatures.find((s: any) => s.construct !== true)
      if (t.fields.some((field: any) => !isIndexField(field)) || call === undefined) return objectNorm(t.fields, inner)
      return caliperNorm({ kind: 'function', ...call }, depth)
    }
    // A member is its value, as the checker states an enum member.
    case 'enumMember': {
      return caliperNorm(t.value, depth)
    }
    // The checker side reads a pattern such as `${number}px` as the strings it stands for.
    case 'templateLiteral': {
      return prim('string')
    }
    // The checker side reads a `UniqueESSymbol` the same way; neither keeps which declaration it belongs to.
    case 'uniqueSymbol': {
      return prim('unique symbol')
    }
    // A reference back to an enclosing type, by its name.
    case 'recursive': {
      return { k: 'name', name: bareName(t.name) }
    }
    // A `readonly` array or tuple is the array or tuple, as the checker reads it.
    case 'readonly': {
      return caliperNorm(t.element, depth)
    }
    case 'function': {
      const params = (t.params ?? []).map((p: any) => ({
        optional: p.optional === true,
        rest: p.rest === true,
        type: inner(p.type),
      }))
      return { k: 'function', params, returns: inner(t.returnType) }
    }
    // `PropType` has no `bigint` primitive yet, so Caliper writes it as the name `bigint`, which no declaration can take
    // (TS2457): it is the primitive.
    case 'named': {
      if (t.name === 'bigint' && (t.args ?? []).length === 0) return prim('bigint')
      const family = omittedFamily(t, inner)
      return { k: 'name', name: bareName(t.name), ...(family && { family }) }
    }
    // `React.HTMLAttributes<…>` and its relatives, with the element and the keys left out that the type carries.
    case 'htmlAttributes': {
      return { k: 'name', name: 'HTMLAttributes', family: { element: t.element, omitted: t.omitted ?? [] } }
    }
    default: {
      return TERMINALS[t.kind] ? { k: 'name', name: TERMINALS[t.kind] } : { k: 'opaque' }
    }
  }
}
