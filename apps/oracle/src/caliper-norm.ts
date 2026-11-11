// Caliper's side of src/oracle-compare.ts: its canonical `PropType` JSON reduced to the shape the comparison reads, as
// the checker side (src/oracle-checker.ts) reduces the checker's type.
import type { ObjectField, PropType } from '../../../packages/napi/index.d.ts'
import { bareName, isBareObject, lit, MAX_DEPTH, prim, type HtmlFamily, type Norm } from './oracle-compare.ts'

const isIndexField = (field: ObjectField) => field.indexKey !== undefined

/** A field's name as the checker side keys it. Caliper writes a member keyed by a symbol as the source does, `[tag]` or
 * `[Symbol.iterator]`; the checker keys it `__@tag@<symbol id>`, which src/oracle-checker.ts reads as `__@tag`. */
export function fieldName(name: string): string {
  const symbol = /^\[(?:Symbol\.)?([A-Za-z_$][\w$]*)\]$/.exec(name)
  return symbol ? `__@${symbol[1]}` : name
}

/** An object type's fields. Caliper writes an index signature as one field whose `indexKey` is the key type: its name is
 * the signature's parameter name (`[id: string]: number`), not a property. */
function objectNorm(fields: ObjectField[], inner: (m: PropType) => Norm): Norm {
  const indexed = fields.filter((field) => isIndexField(field))
  const named = fields.filter((field) => !isIndexField(field))
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
function omittedFamily(name: string, args: PropType[], inner: (m: PropType) => Norm): HtmlFamily | undefined {
  const [attributes, key] = args
  if (bareName(name) !== 'Omit' || args.length !== 2 || attributes.kind !== 'htmlAttributes') return undefined
  const keys = omittedKeys(inner(key))
  return keys && { element: attributes.element, omitted: [...attributes.omitted, ...keys] }
}

/** A `PropType` no case above reads: unreachable while every kind of the published type has one. */
function unreadable(t: never): never {
  throw new Error(`caliperNorm has no case for the PropType ${JSON.stringify(t)}; add one to src/caliper-norm.ts`)
}

/** Caliper's canonical `PropType` JSON, normalized. Every kind of the published `PropType` (packages/napi/index.d.ts)
 * has a case, so a kind added there fails the type check here, and a kind this build does not know throws instead of
 * being compared as opaque. */
export function caliperNorm(t: PropType, depth = 0): Norm {
  if (depth > MAX_DEPTH) return { k: 'opaque' }
  const inner = (m: PropType) => caliperNorm(m, depth + 1)
  switch (t.kind) {
    case 'string':
    case 'number':
    case 'boolean':
    case 'bigint':
    case 'null':
    case 'undefined':
    case 'any':
    case 'unknown':
    case 'never':
    case 'void': {
      return prim(t.kind)
    }
    case 'stringLiteral': {
      return lit(t.value, 'string')
    }
    case 'numberLiteral': {
      return lit(String(t.value), 'number')
    }
    case 'boolLiteral': {
      return lit(String(t.value), 'boolean')
    }
    case 'bigintLiteral': {
      return lit(t.value, 'bigint')
    }
    case 'literalUnion': {
      return { k: 'union', members: t.members.map((m) => lit(m, 'string')) }
    }
    case 'union': {
      return { k: 'union', members: t.members.map((m) => inner(m)) }
    }
    case 'intersection': {
      const parts = t.members.map((m) => inner(m))
      const meaningful = parts.filter((m) => !isBareObject(m))
      return meaningful.length === 1 ? meaningful[0] : { k: 'intersection', members: parts }
    }
    case 'array': {
      return { k: 'array', element: inner(t.element) }
    }
    // An element is `{ type, label?, modifier? }`. The checker side lists a rest element by its element type and has no
    // markers, so neither does this one.
    case 'tuple': {
      return { k: 'tuple', elements: t.elements.map((e) => inner(e.type)) }
    }
    case 'object': {
      return objectNorm(t.fields, inner)
    }
    // As the checker side reads an object type with signatures: its properties when it has any, else its first call
    // signature as a function, else (construct signatures only) an empty object.
    case 'callable': {
      const call = t.signatures.find((s) => s.construct !== true)
      if (t.fields.some((field) => !isIndexField(field)) || call === undefined) return objectNorm(t.fields, inner)
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
      const params = t.params.map((p) => ({
        optional: p.optional === true,
        rest: p.rest === true,
        type: inner(p.type),
      }))
      return { k: 'function', params, returns: inner(t.returnType) }
    }
    case 'named': {
      const family = omittedFamily(t.name, t.args, inner)
      return { k: 'name', name: bareName(t.name), ...(family && { family }) }
    }
    // `React.HTMLAttributes<…>` and its relatives, with the element and the keys left out that the type carries.
    case 'htmlAttributes': {
      return { k: 'name', name: 'HTMLAttributes', family: { element: t.element, omitted: t.omitted } }
    }
    case 'reactNode': {
      return { k: 'name', name: 'ReactNode' }
    }
    case 'cssProperties': {
      return { k: 'name', name: 'CSSProperties' }
    }
    case 'elementType': {
      return { k: 'name', name: 'ElementType' }
    }
    case 'ref': {
      return { k: 'name', name: 'Ref' }
    }
    case 'sxProps': {
      return { k: 'name', name: 'SxProps' }
    }
    // Caliper did not evaluate these, and says so: its own `opaque`, and a type cut at a depth or member limit.
    case 'opaque':
    case 'truncated': {
      return { k: 'opaque' }
    }
  }
  return unreadable(t)
}
