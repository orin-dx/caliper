// The structural comparison behind src/oracle.ts: both sides' prop types reduced to one shape, and one verdict per
// pair. Pure: nothing here talks to the checker.

/** A type reduced to what both sides can state: kind, literal value, members, fields, parameters. `alias` is the
 * checker's name for it (an alias, or the interface that declares it), which Caliper may print instead. */
export type Norm = (
  | { k: 'prim'; name: string }
  | { k: 'lit'; value: string }
  | { k: 'union'; members: Norm[] }
  | { k: 'intersection'; members: Norm[] }
  | { k: 'array'; element: Norm }
  | { k: 'tuple'; elements: Norm[] }
  | { k: 'function'; params: { optional: boolean; rest: boolean; type: Norm }[]; returns: Norm }
  | { k: 'object'; fields: string[]; index: boolean }
  | { k: 'name'; name: string }
  | { k: 'param'; name: string; fallback?: Norm; constraint?: Norm }
  | { k: 'opaque' }
  | { k: 'error' }
) & { alias?: string }

export type Verdict = 'identical' | 'spelled' | 'less precise' | 'wrong' | 'checker any/error'

export const MAX_DEPTH = 3
export const VERDICTS: Verdict[] = ['identical', 'spelled', 'less precise', 'wrong', 'checker any/error']
const RANK: Verdict[] = ['identical', 'spelled', 'less precise', 'wrong']

/** A type name without `React.` or a namespace qualifier. */
export const bareName = (name: string) => name.replace(/^.*\./, '')
export const prim = (name: string): Norm => ({ k: 'prim', name })

const TERMINALS: Record<string, string> = {
  reactNode: 'ReactNode',
  cssProperties: 'CSSProperties',
  elementType: 'ElementType',
  htmlAttributes: 'HTMLAttributes',
  ref: 'Ref',
  sxProps: 'SxProps',
}
const PRIMITIVES = new Set(['string', 'number', 'boolean', 'null', 'undefined', 'any', 'unknown', 'never', 'void'])
const LITERALS = new Set(['stringLiteral', 'numberLiteral', 'boolLiteral'])

/** Caliper's canonical `PropType` JSON, normalized. */
export function caliperNorm(t: any, depth = 0): Norm {
  if (!t || depth > MAX_DEPTH) return { k: 'opaque' }
  if (PRIMITIVES.has(t.kind)) return prim(t.kind)
  if (LITERALS.has(t.kind)) return { k: 'lit', value: String(t.value) }
  const inner = (m: any) => caliperNorm(m, depth + 1)
  switch (t.kind) {
    case 'literalUnion': {
      return { k: 'union', members: t.members.map((m: string) => ({ k: 'lit', value: m })) }
    }
    case 'union': {
      return { k: 'union', members: t.members.map((m: any) => inner(m)) }
    }
    case 'intersection': {
      const parts: Norm[] = t.members.map((m: any) => inner(m))
      const meaningful = parts.filter((m) => !(m.k === 'object' && m.fields.length === 0))
      return meaningful.length === 1 ? meaningful[0] : { k: 'intersection', members: parts }
    }
    case 'array': {
      return { k: 'array', element: inner(t.element) }
    }
    case 'tuple': {
      return { k: 'tuple', elements: (t.elements ?? t.members ?? []).map((m: any) => inner(m)) }
    }
    case 'object': {
      return { k: 'object', fields: t.fields.map((field: any) => field.name).toSorted(), index: false }
    }
    case 'function': {
      const params = (t.params ?? []).map((p: any) => ({ optional: false, rest: false, type: inner(p.type) }))
      return { k: 'function', params, returns: inner(t.returnType) }
    }
    case 'named': {
      return { k: 'name', name: bareName(t.name) }
    }
    default: {
      return TERMINALS[t.kind] ? { k: 'name', name: TERMINALS[t.kind] } : { k: 'opaque' }
    }
  }
}

const worst = (vs: Verdict[]): Verdict => vs.reduce((a, b) => (RANK.indexOf(b) > RANK.indexOf(a) ? b : a), 'identical')
const best = (vs: Verdict[]): Verdict => vs.reduce((a, b) => (RANK.indexOf(b) < RANK.indexOf(a) ? b : a), 'wrong')
const isLit = (n: Norm, value?: string) => n.k === 'lit' && (value === undefined || n.value === value)
const isUndefined = (m: Norm) => m.k === 'prim' && m.name === 'undefined'
const WIDENING = new Set(['string', 'number', 'boolean'])

/** Union members without `undefined` (an optional prop's, which both sides treat as optional), flattened, with
 * `true | false` as `boolean`. */
function members(n: Norm): Norm[] {
  const flat: Norm[] = []
  const push = (m: Norm) => {
    if (m.k === 'union') m.members.forEach(push)
    // An optional generic prop reads `T | (undefined & T)`.
    else if (!isUndefined(m) && !(m.k === 'intersection' && m.members.some(isUndefined))) flat.push(m)
  }
  push(n)
  if (flat.some((m) => isLit(m, 'true')) && flat.some((m) => isLit(m, 'false'))) {
    return [...flat.filter((m) => !isLit(m, 'true') && !isLit(m, 'false')), prim('boolean')]
  }
  return flat
}

/** Two unions: each checker member covered by one of Caliper's (a missing member makes Caliper's type too narrow),
 * and each of Caliper's matched by one of the checker's (an extra member makes it too wide, unless it widens
 * literals). */
function compareUnions(cm: Norm[], km: Norm[], depth: number): Verdict {
  const kCovered = km.map((m) => best(cm.map((cMember) => compare(cMember, m, depth + 1))))
  const cMatched = cm.map((m) => {
    const verdict = best(km.map((kMember) => compare(m, kMember, depth + 1)))
    const widens =
      m.k === 'prim' &&
      WIDENING.has(m.name) &&
      km.some((kMember) => kMember.k === 'lit' || (kMember.k === 'prim' && kMember.name === m.name))
    return verdict === 'wrong' && widens ? 'less precise' : verdict
  })
  return worst([...kCovered, ...cMatched])
}

function compareFunctions(c: Norm & { k: 'function' }, k: Norm & { k: 'function' }, depth: number): Verdict {
  const verdicts: Verdict[] = [compare(c.returns, k.returns, depth + 1)]
  for (const [i, kp] of k.params.entries()) {
    const cp = c.params[i]
    if (cp === undefined) {
      verdicts.push(kp.optional || kp.rest ? 'less precise' : 'wrong')
    } else {
      verdicts.push(compare(cp.type, kp.type, depth + 1))
      // Caliper's function types record neither marker.
      if (kp.optional || kp.rest) verdicts.push('less precise')
    }
  }
  if (c.params.length > k.params.length) verdicts.push('wrong')
  return worst(verdicts)
}

/** One non-union type against another. */
function compareOne(c: Norm, k: Norm, depth: number): Verdict {
  switch (k.k) {
    case 'prim': {
      return c.k === 'prim' && c.name === k.name ? 'identical' : 'wrong'
    }
    case 'lit': {
      if (c.k === 'lit') return c.value === k.value ? 'identical' : 'wrong'
      return c.k === 'prim' && WIDENING.has(c.name) ? 'less precise' : 'wrong'
    }
    case 'array': {
      return c.k === 'array' ? compare(c.element, k.element, depth + 1) : 'wrong'
    }
    case 'tuple': {
      if (c.k !== 'tuple') return c.k === 'array' ? 'less precise' : 'wrong'
      if (c.elements.length !== k.elements.length) return 'wrong'
      return worst(c.elements.map((e, i) => compare(e, k.elements[i], depth + 1)))
    }
    case 'function': {
      return c.k === 'function' ? compareFunctions(c, k, depth) : 'wrong'
    }
    case 'object': {
      if (c.k !== 'object') return c.k === 'prim' && c.name === 'object' ? 'less precise' : 'wrong'
      const own = new Set(c.fields)
      if (c.fields.some((f) => !k.fields.includes(f))) return 'wrong'
      return k.fields.some((f) => !own.has(f)) || k.index ? 'less precise' : 'identical'
    }
    case 'intersection': {
      if (c.k !== 'intersection') return 'less precise'
      return c.members.length === k.members.length ? 'identical' : 'less precise'
    }
    case 'param': {
      // A type parameter of the component itself, which Caliper replaced with an instantiation: its default, as a bare
      // `AccordionProps` means `AccordionProps<false>`, or its constraint. That is one instance of the parameter,
      // looser or narrower than the parameter itself, and not another type; a type that is neither instance is.
      // `c` may be one member of a union Caliper wrote out (`string` of `string | number`), so it must fit one member of
      // an instance, not all of them.
      const instances = [k.fallback, k.constraint].filter((i): i is Norm => i !== undefined).flatMap((i) => members(i))
      return instances.some((i) => compare(c, i, depth + 1) !== 'wrong') ? 'less precise' : 'wrong'
    }
    default: {
      return 'wrong'
    }
  }
}

/** How Caliper's normalized type `c` stands against the checker's `k`. */
export function compare(c: Norm, k: Norm, depth = 0): Verdict {
  if (depth > MAX_DEPTH) return 'identical'
  // The checker has no type to compare with: an error, `any`, or a top-level `unknown` where Caliper has a type
  // (most often a declaration the program couldn't resolve).
  const uninformative =
    k.k === 'error' || (k.k === 'prim' && k.name === 'any') || (depth === 0 && k.k === 'prim' && k.name === 'unknown')
  if (uninformative && !(c.k === 'prim' && k.k === 'prim' && c.name === k.name)) {
    return depth === 0 ? 'checker any/error' : 'identical'
  }
  const km = members(k)
  // An optional prop of type `never` reads `undefined` in the checker, which `members` drops as the optional marker.
  if (km.length === 0 && c.k === 'prim' && c.name === 'never') return 'identical'
  if (c.k === 'name') {
    // An optional prop's checker type is `X | undefined`; the name is X's.
    const target = km.length === 1 ? km[0] : k
    if ((target.alias && bareName(target.alias) === c.name) || (k.alias && bareName(k.alias) === c.name)) {
      return 'spelled'
    }
    if (target.k === 'param' && target.name === c.name) return 'identical'
    // A name Caliper left unexpanded where the checker has the type itself (an unbound parameter, an alias that
    // resolves to another name).
    return 'less precise'
  }
  if (k.k === 'opaque') return 'identical'
  if (c.k === 'opaque') return 'less precise'
  if (c.k === 'prim' && (c.name === 'any' || c.name === 'unknown') && !(k.k === 'prim' && k.name === c.name)) {
    return 'less precise'
  }
  // An intersection Caliper left as written, which the checker reduced (`Mode & …` to a literal union).
  if (c.k === 'intersection' && !km.some((m) => m.k === 'intersection')) return 'less precise'
  const cm = members(c)
  if (cm.length !== 1 || km.length !== 1) return compareUnions(cm, km, depth)
  const [cOne] = cm
  const [kOne] = km
  if (cOne !== c || kOne !== k) return compare(cOne, { ...kOne, alias: kOne.alias ?? k.alias }, depth)
  return compareOne(c, k, depth)
}
