import type { Few, Value } from 'fake-dep'

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
