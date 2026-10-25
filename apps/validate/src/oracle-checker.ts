// The checker side of src/oracle.ts: a TypeScript 7.1 type reduced to the shape src/oracle-compare.ts compares,
// once per type id and depth.
import { SignatureKind, SymbolFlags, TypeFlags } from 'typescript-beta/sync'
import { MAX_DEPTH, prim, type Norm } from './oracle-compare.ts'

const ANONYMOUS = new Set(['__type', '__object', '__function', '__call', ''])

const INTRINSICS: [number, string][] = [
  [TypeFlags.Any, 'any'],
  [TypeFlags.Unknown, 'unknown'],
  [TypeFlags.String, 'string'],
  [TypeFlags.Number, 'number'],
  [TypeFlags.BigInt, 'bigint'],
  [TypeFlags.ESSymbol, 'symbol'],
  [TypeFlags.Null, 'null'],
  [TypeFlags.Undefined, 'undefined'],
  [TypeFlags.Void, 'void'],
  [TypeFlags.Never, 'never'],
  [TypeFlags.NonPrimitive, 'object'],
  // A pattern such as `var(--${string})` or `Uppercase<string>` stands for strings.
  [TypeFlags.TemplateLiteral | TypeFlags.StringMapping, 'string'],
]

/** The checker's name for a type: its alias, or the interface, class or enum that declares it. */
function typeName(t: any): string | undefined {
  const alias = t.getAliasSymbol()
  if (alias && !ANONYMOUS.has(alias.name)) return alias.name
  if (t.flags & (TypeFlags.Object | TypeFlags.Enum | TypeFlags.EnumLiteral)) {
    const symbol = t.getSymbol()
    if (symbol && !ANONYMOUS.has(symbol.name)) return symbol.name
  }
  return undefined
}

/** A normalizer bound to one checker, caching by type id and depth. */
export function checkerNormalizer(checker: any): (t: any) => Norm {
  const cache = new Map<string, Norm>()

  function norm(t: any, depth = 0): Norm {
    const key = `${t.id}:${depth}`
    const cached = cache.get(key)
    if (cached) return cached
    const result = uncached(t, depth)
    cache.set(key, result)
    return result
  }

  function scalar(t: any): Norm | undefined {
    const f: number = t.flags
    if (t.isErrorType()) return { k: 'error' }
    if (f & TypeFlags.Boolean) return { ...prim('boolean'), alias: typeName(t) }
    const intrinsic = INTRINSICS.find(([flag]) => f & flag)
    if (intrinsic) return prim(intrinsic[1])
    // The beta's responses carry neither `value` nor `intrinsicName` for `true` and `false`.
    if (f & TypeFlags.BooleanLiteral) return { k: 'lit', value: checker.typeToString(t) }
    if (f & (TypeFlags.StringLiteral | TypeFlags.NumberLiteral | TypeFlags.BigIntLiteral)) {
      return { k: 'lit', value: String(t.value) }
    }
    return undefined
  }

  /** A type parameter with what Caliper may have put in its place: its default, and its constraint. */
  const expanding = new Set<number>()
  function typeParameter(t: any, depth: number): Norm {
    const name: string = t.getSymbol()?.name ?? ''
    // A parameter inside its own default or constraint (`T extends Foo<T>`) is its name.
    if (expanding.has(t.id)) return { k: 'param', name }
    expanding.add(t.id)
    // Its instances sit one level down, but never past the depth limit, where a scalar one still reads.
    const at = Math.min(depth + 1, MAX_DEPTH)
    const read = (get: () => any): Norm | undefined => {
      try {
        const type = get()
        return type ? norm(type, at) : undefined
      } catch {
        return undefined
      }
    }
    try {
      return {
        k: 'param',
        name,
        fallback: read(() => checker.getDefaultFromTypeParameter(t)),
        constraint: read(() => checker.getBaseConstraintOfType(t)),
      }
    } finally {
      expanding.delete(t.id)
    }
  }

  function union(t: any, depth: number, alias: string | undefined): Norm {
    const parts: Norm[] = (t.getTypes() ?? []).map((m: any) => norm(m, depth + 1))
    // An optional prop's `StyleProp<X> | undefined` is a new union without an alias; the type without `undefined`
    // keeps it, as TypeScript prints it.
    const optionalAlias =
      alias === undefined && parts.some((m) => m.k === 'prim' && m.name === 'undefined')
        ? typeName(checker.getNonNullableType(t))
        : undefined
    return { k: 'union', members: parts, alias: alias ?? optionalAlias }
  }

  function intersection(t: any, depth: number, alias: string | undefined): Norm {
    const parts: Norm[] = (t.getTypes() ?? []).map((m: any) => norm(m, depth + 1))
    // `string & {}` (the keep-the-literals trick) is a string.
    const meaningful = parts.filter((m) => !(m.k === 'object' && m.fields.length === 0 && !m.index))
    if (meaningful.length === 1) return { ...meaningful[0], alias: alias ?? meaningful[0].alias }
    return { k: 'intersection', members: parts, alias }
  }

  function object(t: any, depth: number, alias: string | undefined): Norm {
    if (checker.isArrayType(t)) {
      const [element] = checker.getTypeArguments(t)
      return { k: 'array', element: element ? norm(element, depth + 1) : { k: 'opaque' }, alias }
    }
    if (checker.isTupleType(t)) {
      return { k: 'tuple', elements: checker.getTypeArguments(t).map((e: any) => norm(e, depth + 1)), alias }
    }
    const properties = checker.getPropertiesOfType(t)
    const [sig] = checker.getSignaturesOfType(t, SignatureKind.Call)
    if (sig && properties.length === 0) {
      const params = sig.getParameters()
      const paramTypes = params.length > 0 ? checker.getTypeOfSymbol(params) : []
      return {
        k: 'function',
        params: params.map((p: any, i: number) => ({
          optional: Boolean(p.flags & SymbolFlags.Optional),
          rest: sig.hasRestParameter && i === params.length - 1,
          type: norm(paramTypes[i], depth + 1),
        })),
        returns: norm(checker.getReturnTypeOfSignature(sig), depth + 1),
        alias,
      }
    }
    const index = checker.getIndexInfosOfType(t).length > 0
    return { k: 'object', fields: properties.map((p: any) => p.name).toSorted(), index, alias }
  }

  function uncached(t: any, depth: number): Norm {
    if (t.flags & TypeFlags.TypeParameter) return typeParameter(t, depth)
    const known = scalar(t)
    if (known) return known
    const alias = typeName(t)
    if (depth >= MAX_DEPTH) return { k: 'opaque', alias }
    if (t.flags & TypeFlags.Union) return union(t, depth, alias)
    if (t.flags & TypeFlags.Intersection) return intersection(t, depth, alias)
    if (t.flags & TypeFlags.Object) return object(t, depth, alias)
    return { k: 'opaque', alias }
  }

  return (t: any) => norm(t)
}
