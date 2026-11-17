import type { Few, Theirs, Value } from 'fake-dep'

// As many keys as the dependency's `Conditions`, but declared here: the project's own properties are never cut.
interface Mine {
  m01?: boolean;
  m02?: boolean;
  m03?: boolean;
  m04?: boolean;
  m05?: boolean;
  m06?: boolean;
  m07?: boolean;
  m08?: boolean;
  m09?: boolean;
  m10?: boolean;
  m11?: boolean;
  m12?: boolean;
  m13?: boolean;
  m14?: boolean;
  m15?: boolean;
  m16?: boolean;
  m17?: boolean;
  m18?: boolean;
  m19?: boolean;
  m20?: boolean;
  m21?: boolean;
  m22?: boolean;
  m23?: boolean;
  m24?: boolean;
  m25?: boolean;
  m26?: boolean;
  m27?: boolean;
  m28?: boolean;
  m29?: boolean;
  m30?: boolean;
  m31?: boolean;
  m32?: boolean;
  m33?: boolean;
  m34?: boolean;
  m35?: boolean;
  m36?: boolean;
  m37?: boolean;
  m38?: boolean;
  m39?: boolean;
  m40?: boolean;
  m41?: boolean;
  m42?: boolean;
  m43?: boolean;
  m44?: boolean;
  m45?: boolean;
}

type Own<V> = V | { [K in keyof Mine]?: V }

/** Three maps over one constraint, none of it the project's: one list of properties answers for all of them. */
export function Shared(props: { a: Value<string>; b: Value<number>; c: Value<boolean> }) {
  return null
}

/** Two maps over properties the project declares: each is listed, whole. */
export function Owned(props: { a: Own<string>; b: Own<number> }) {
  return null
}

/** Two maps within the budget: each is listed with its own types. */
export function Small(props: { a: Few<string>; b: Few<number> }) {
  return null
}

// The keys of the dependency's `Theirs`, declared here.
interface Ours {
  k01?: boolean;
  k02?: boolean;
  k03?: boolean;
  k04?: boolean;
  k05?: boolean;
  k06?: boolean;
  k07?: boolean;
  k08?: boolean;
  k09?: boolean;
  k10?: boolean;
  k11?: boolean;
  k12?: boolean;
  k13?: boolean;
  k14?: boolean;
  k15?: boolean;
  k16?: boolean;
  k17?: boolean;
  k18?: boolean;
  k19?: boolean;
  k20?: boolean;
  k21?: boolean;
  k22?: boolean;
  k23?: boolean;
  k24?: boolean;
  k25?: boolean;
  k26?: boolean;
  k27?: boolean;
  k28?: boolean;
  k29?: boolean;
  k30?: boolean;
  k31?: boolean;
  k32?: boolean;
  k33?: boolean;
  k34?: boolean;
  k35?: boolean;
  k36?: boolean;
  k37?: boolean;
  k38?: boolean;
  k39?: boolean;
  k40?: boolean;
  k41?: boolean;
  k42?: boolean;
  k43?: boolean;
  k44?: boolean;
  k45?: boolean;
}

// The keys written out, so that one constraint is one type whichever interface they are the keys of.
type Keys =
  | "k01" | "k02" | "k03" | "k04" | "k05" | "k06" | "k07" | "k08" | "k09" | "k10" | "k11" | "k12" | "k13" | "k14" | "k15"
  | "k16" | "k17" | "k18" | "k19" | "k20" | "k21" | "k22" | "k23" | "k24" | "k25" | "k26" | "k27" | "k28" | "k29" | "k30"
  | "k31" | "k32" | "k33" | "k34" | "k35" | "k36" | "k37" | "k38" | "k39" | "k40" | "k41" | "k42" | "k43" | "k44" | "k45"

// Over a key type parameter, the properties are those of `T` named by the keys: declared where `T`'s are.
type Picked<T, K extends keyof T> = null | { [P in K]?: T[P] }

/** One map written over a type parameter, given a dependency's type and then the project's, with the same keys. */
export function Generic(props: { a: Picked<Theirs, Keys>; b: Picked<Ours, Keys> }) {
  return null
}
