// The structural comparison behind src/oracle.ts: both sides' prop types reduced to one shape, and one verdict per
// pair. Pure: nothing here talks to the checker.
import { HTML_ATTRIBUTE_MEMBERS } from './html-attribute-members.ts'
import { SEVERITY, type Verdict } from './vocabulary.ts'

/** The primitive a literal is a value of: `"0"` and `0` spell the same `value` and are two types. */
export type LiteralKind = 'string' | 'number' | 'bigint' | 'boolean'

/** A name that is a recognized React HTML-attributes family: the attributes of `element`, less the `omitted` keys
 * (`HTMLAttributes<HTMLDivElement>`, or `Omit<…, "color">` of it). The name stays what it is for every comparison
 * that reads names; only the intersection rule below reads the family. */
export interface HtmlFamily {
  element: string
  omitted: string[]
}

/** A type reduced to what both sides can state: kind, literal value, members, fields, parameters. `alias` is the
 * checker's name for it (an alias, or the interface that declares it), which Caliper may print instead. */
export type Norm = (
  | { k: 'prim'; name: string }
  | { k: 'lit'; value: string; of: LiteralKind }
  | { k: 'union'; members: Norm[] }
  | { k: 'intersection'; members: Norm[] }
  | { k: 'array'; element: Norm }
  | { k: 'tuple'; elements: Norm[] }
  | { k: 'function'; params: { optional: boolean; rest: boolean; type: Norm }[]; returns: Norm }
  | { k: 'object'; fields: string[]; index: boolean; indexValue?: Norm }
  | { k: 'name'; name: string; family?: HtmlFamily }
  | { k: 'param'; name: string; fallback?: Norm; constraint?: Norm }
  | { k: 'opaque' }
  | { k: 'error' }
) & { alias?: string }

export const MAX_DEPTH = 3

/** A type name without `React.` or a namespace qualifier. */
export const bareName = (name: string) => name.replace(/^.*\./, '')
export const prim = (name: string): Norm => ({ k: 'prim', name })
export const lit = (value: string, of: LiteralKind): Norm => ({ k: 'lit', value, of })

/** `{}`: an object with no property and no index signature. The one place that decides it, for every reader of a
 * `Norm`: an index-only object (`{ [key: string]: V }`) has no fields either, and is not empty. */
export const isBareObject = (m: Norm) => m.k === 'object' && m.fields.length === 0 && !m.index

const rank = (v: Verdict) => SEVERITY.indexOf(v)
const worst = (vs: Verdict[]): Verdict => vs.reduce((a, b) => (rank(b) > rank(a) ? b : a), 'identical')
const best = (vs: Verdict[]): Verdict => vs.reduce((a, b) => (rank(b) < rank(a) ? b : a), 'wrong')
const isBool = (n: Norm, value: string) => n.k === 'lit' && n.of === 'boolean' && n.value === value
const isUndefined = (m: Norm) => m.k === 'prim' && m.name === 'undefined'
const WIDENING = new Set(['string', 'number', 'boolean'])

/** An empty object type, or `Record<…>` (its arguments are not kept): what `string & {}` intersects with. */
const isEmptyObject = (m: Norm) => isBareObject(m) || (m.k === 'name' && m.name === 'Record')

/** Whether `m` is the primitive `name`, or `name & {}`, or a union with either. */
function absorbs(m: Norm, name: string): boolean {
  if (m.k === 'prim') return m.name === name
  if (m.k === 'union') return m.members.some((member) => absorbs(member, name))
  return m.k === 'intersection' && m.members.some(isEmptyObject) && m.members.some((member) => absorbs(member, name))
}

/** A name or opaque type that Caliper (or the checker) did not expand: what it holds is unknown, so no member beside it
 * can prove the other side wrong. */
const isUnexpanded = (m: Norm) => m.k === 'name' || m.k === 'opaque'

/** Union members without `undefined` (an optional prop's, which both sides treat as optional), flattened, with
 * `true | false` as `boolean`. */
function members(n: Norm, coverLiterals = false): Norm[] {
  const flat: Norm[] = []
  const push = (m: Norm) => {
    if (m.k === 'union') m.members.forEach(push)
    // An optional generic prop reads `T | (undefined & T)`.
    else if (!isUndefined(m) && !(m.k === 'intersection' && m.members.some(isUndefined))) flat.push(m)
  }
  push(n)
  // `'default' | 'large' | string` is `string` to the checker: a literal that a primitive of its kind in the same
  // union already covers adds nothing, so a union written with it holds the same values as one without. The checker's
  // `"blue" | … | (string & {})` keeps its literals for the reader, so `compare` scores it spelled, not identical. A
  // literal is covered by the primitive of its own kind only: `"0"` is not covered by `number`, `true` not by `string`.
  // `string & {}` and `string & Record<never, never>` (the `LiteralUnion` spellings) are `string` for this purpose.
  const has = (name: string) => flat.some((m) => absorbs(m, name))
  const covered = (m: Norm) => coverLiterals && m.k === 'lit' && has(m.of)
  const kept = flat.filter((m) => !covered(m))
  if (kept.some((m) => isBool(m, 'true')) && kept.some((m) => isBool(m, 'false'))) {
    return [...kept.filter((m) => !isBool(m, 'true') && !isBool(m, 'false')), prim('boolean')]
  }
  return kept
}

/** The members of React's `ReactNode` (the `bigint` and `Promise` of React 19 when there), by kind and by the
 * checker's name for each object. */
const REACT_NODE_PRIMS = ['null', 'string', 'number', 'boolean']
const REACT_NODE_OBJECTS = ['ReactElement', 'ReactPortal', 'Iterable']
const isReactNodeMember = (m: Norm) =>
  (m.k === 'prim' && [...REACT_NODE_PRIMS, 'bigint'].includes(m.name)) ||
  (m.k === 'object' && m.alias !== undefined && [...REACT_NODE_OBJECTS, 'Promise'].includes(m.alias))

/** The checker's union members with `ReactNode`'s own taken back into one union named `ReactNode`, when Caliper's union
 * names `ReactNode` and the checker's holds all of its members: TypeScript keeps the alias only on a union of exactly
 * them, so `ReactNode | ((open: boolean) => ReactNode)` arrives flattened. Unchanged otherwise. */
function withReactNode(cm: Norm[], km: Norm[]): Norm[] {
  if (!cm.some((m) => m.k === 'name' && m.name === 'ReactNode')) return km
  const span = km.filter((m) => isReactNodeMember(m))
  const has = (m: Norm) =>
    span.some(
      (s) => (s.k === 'prim' && m.k === 'prim' && s.name === m.name) || (s.k === 'object' && s.alias === m.alias),
    )
  const required: Norm[] = [
    ...REACT_NODE_PRIMS.map((name) => prim(name)),
    ...REACT_NODE_OBJECTS.map((alias): Norm => ({ k: 'object', fields: [], index: false, alias })),
  ]
  if (!required.every((m) => has(m))) return km
  return [...km.filter((m) => !isReactNodeMember(m)), { k: 'union', members: span, alias: 'ReactNode' }]
}

/** Two unions: each checker member covered by one of Caliper's (a missing member makes Caliper's type too narrow),
 * and each of Caliper's matched by one of the checker's (an extra member makes it too wide, unless it widens
 * literals). */
function compareUnions(cm: Norm[], checkerMembers: Norm[], depth: number): Verdict {
  const km = withReactNode(cm, checkerMembers)
  const kCovered = km.map((m) => best(cm.map((cMember) => compare(cMember, m, depth + 1))))
  const cMatched = cm.map((m) => {
    const verdict = best(km.map((kMember) => compare(m, kMember, depth + 1)))
    const widens =
      m.k === 'prim' &&
      WIDENING.has(m.name) &&
      km.some((kMember) => kMember.k === 'lit' || (kMember.k === 'prim' && kMember.name === m.name))
    return verdict === 'wrong' && widens ? 'less precise' : verdict
  })
  const verdict = worst([...kCovered, ...cMatched])
  // A union with a name or opaque member (`MantineColor | 'initials'`, a template literal) may hold the other side's
  // values in that member, so a member it cannot match is not proof of a different type.
  return verdict === 'wrong' && [...cm, ...km].some((m) => isUnexpanded(m)) ? 'less precise' : verdict
}

/** An intersection of one object literal and one recognized HTML-attributes family (`{ color?; as? } & Omit<
 * HTMLAttributes<div>, "color">`), against the checker's one flat object. The checker flattens the intersection, so the
 * object holds the literal's own members plus the family's members less the omitted keys. Equal sets are the same type;
 * a member the flat object lacks is wrong, and one it has beyond them (an omitted key it still has, a member the family
 * does not) leaves the intersection less precise, as every intersection Caliper left as written is. A family with no
 * member table, or an intersection of any other shape, is left as written. */
function flattenedFamily(c: Norm & { k: 'intersection' }, flat: Norm & { k: 'object' }): Verdict {
  const [first, second] = c.members
  const literal = first.k === 'object' ? first : second
  const named = first.k === 'object' ? second : first
  const family = named.k === 'name' ? named.family : undefined
  const familyMembers = family && HTML_ATTRIBUTE_MEMBERS[family.element]
  if (c.members.length !== 2 || literal.k !== 'object' || !family || !familyMembers || literal.index || flat.index) {
    return 'less precise'
  }
  const omitted = new Set(family.omitted)
  const expected = new Set([...literal.fields, ...familyMembers.filter((name) => !omitted.has(name))])
  const actual = new Set(flat.fields)
  if ([...expected].some((name) => !actual.has(name))) return 'wrong'
  return actual.size === expected.size ? 'identical' : 'less precise'
}

function compareFunctions(c: Norm & { k: 'function' }, k: Norm & { k: 'function' }, depth: number): Verdict {
  const verdicts: Verdict[] = [compare(c.returns, k.returns, depth + 1)]
  for (const [i, kp] of k.params.entries()) {
    const cp = c.params[i]
    if (cp === undefined) {
      verdicts.push(kp.optional || kp.rest ? 'less precise' : 'wrong')
    } else {
      verdicts.push(compare(cp.type, kp.type, depth + 1))
      // A marker the checker has and this side lost.
      if ((kp.optional && !cp.optional) || (kp.rest && !cp.rest)) verdicts.push('less precise')
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
      if (c.k === 'lit') return c.value === k.value && c.of === k.of ? 'identical' : 'wrong'
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
      // An index signature Caliper has and the checker does not describes a different type.
      if (c.index && !k.index) return 'wrong'
      const verdicts: Verdict[] = [
        k.fields.some((f) => !own.has(f)) || (k.index && !c.index) ? 'less precise' : 'identical',
      ]
      // Both state an index: its value type is compared, and one a side does not state is not proof of agreement.
      if (c.index && k.index) {
        verdicts.push(c.indexValue && k.indexValue ? compare(c.indexValue, k.indexValue, depth + 1) : 'less precise')
      }
      return worst(verdicts)
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
      // A parameter with neither (a generic component cast over `forwardRef`, whose props interface declares the
      // default) leaves the checker no instance to hold Caliper's to: any one it picked is one value of the parameter.
      if (instances.length === 0) return 'less precise'
      return instances.some((i) => compare(c, i, depth + 1) !== 'wrong') ? 'less precise' : 'wrong'
    }
    default: {
      return 'wrong'
    }
  }
}

/** How Caliper's normalized type `c` stands against the checker's `k`. A checker union that spells literals a
 * primitive in it already covers (`'a' | 'b' | (string & {})`) holds the values Caliper's `string` does, spelled
 * another way. */
export function compare(c: Norm, k: Norm, depth = 0): Verdict {
  const verdict = compareValues(c, k, depth)
  return verdict === 'identical' && members(k).length !== members(k, true).length ? 'spelled' : verdict
}

function compareValues(c: Norm, k: Norm, depth: number): Verdict {
  if (depth > MAX_DEPTH) return 'not judged'
  // The checker has no type to compare with: an error, `any`, or a top-level `unknown` where Caliper has a type
  // (most often a declaration the program couldn't resolve).
  const uninformative =
    k.k === 'error' || (k.k === 'prim' && k.name === 'any') || (depth === 0 && k.k === 'prim' && k.name === 'unknown')
  if (uninformative && !(c.k === 'prim' && k.k === 'prim' && c.name === k.name)) {
    return depth === 0 ? 'checker any/error' : 'identical'
  }
  const km = members(k, true)
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
  // A type the checker side stopped short of holds whatever Caliper wrote, so agreement is not what it says.
  if (k.k === 'opaque') return 'not judged'
  if (c.k === 'opaque') return 'less precise'
  if (c.k === 'prim' && (c.name === 'any' || c.name === 'unknown') && !(k.k === 'prim' && k.name === c.name)) {
    return 'less precise'
  }
  // An intersection Caliper left as written, which the checker reduced (`Mode & …` to a literal union).
  if (c.k === 'intersection' && !km.some((m) => m.k === 'intersection')) {
    const [flat] = km
    return km.length === 1 && flat.k === 'object' ? flattenedFamily(c, flat) : 'less precise'
  }
  const cm = members(c, true)
  if (cm.length !== 1 || km.length !== 1) return compareUnions(cm, km, depth)
  const [cOne] = cm
  const [kOne] = km
  if (cOne !== c || kOne !== k) return compare(cOne, { ...kOne, alias: kOne.alias ?? k.alias }, depth)
  return compareOne(c, k, depth)
}
